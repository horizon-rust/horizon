//! What sound physics costs, and on which thread (`docs/sound-physics.md`).
//! Usage: `cargo run --release -p horizon-client --example sound_physics_bench [SOUNDS]`.
//!
//! - the game thread: the per-tick section sync (`Acoustics::sync`) over the 9 x 9 chunks
//!   around the listener, and building the material table once;
//! - the sound physics thread: one `evaluate` per sound, in synthetic scenes (open ground, a
//!   stone room with a door, a sound behind a thick wall, a large cave), for ray counts and
//!   bounces around the defaults;
//! - the audio thread: the mixer with and without the filters and reverb buses.
use horizon_client::acoustics::{Acoustics, BlockState, RANGE, Sink, Snapshot, materials};
use horizon_client::chunk::{MIN_Y, SECTIONS, Section, index};
use horizon_client::fast_hash::FxHashMap;
use horizon_client::sound_engine::ChannelId;
use horizon_client::sound_physics::{
    self, Environment, FluidKind, Material, Request, Settings, StateInfo,
};
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

const AIR: u32 = 0;
const STONE: u32 = 1;
const WOOL: u32 = 2;

fn scene_materials() -> Arc<[Material]> {
    let s = Settings::default();
    let state = |block: &str, sound: &str, collides| {
        sound_physics::material(
            &StateInfo {
                block,
                break_sound: &format!("minecraft:block.{sound}.break"),
                step_sound: &format!("minecraft:block.{sound}.step"),
                fluid: FluidKind::None,
                collides,
                sturdy: 0b11_1111,
            },
            &s,
        )
    };
    vec![
        Material::AIR,
        state("minecraft:stone", "stone", true),
        state("minecraft:white_wool", "wool", true),
    ]
    .into()
}
/// The sections of the 9 x 9 chunks around (0, 0), filled by `block(x, y, z)`.
fn world(block: impl Fn(i32, i32, i32) -> u32) -> FxHashMap<(i32, i32, i32), Arc<Section>> {
    let mut sections = FxHashMap::default();
    for cx in -RANGE..=RANGE {
        for cz in -RANGE..=RANGE {
            for sy in 0..SECTIONS as i32 {
                let mut s = Section::default();
                let y0 = MIN_Y + sy * 16;
                for y in y0..y0 + 16 {
                    for z in cz * 16..cz * 16 + 16 {
                        for x in cx * 16..cx * 16 + 16 {
                            let v = block(x, y, z);
                            if v != AIR {
                                s.set(index(x, y, z), v);
                            }
                        }
                    }
                }
                sections.insert((cx, sy, cz), Arc::new(s));
            }
        }
    }
    sections
}
struct Scene {
    name: &'static str,
    snapshot: Snapshot,
    sound: [f64; 3],
    listener: [f64; 3],
}
fn scenes() -> Vec<Scene> {
    let m = scene_materials();
    let ground = |_: i32, y: i32, _: i32| if y < 64 { STONE } else { AIR };
    let room = move |x: i32, y: i32, z: i32| {
        let wall = (-6..=6).contains(&x)
            && (64..=70).contains(&y)
            && (-6..=6).contains(&z)
            && (x.abs() == 6 || z.abs() == 6 || y == 70);
        let door = z == -6 && x == 0 && y <= 65;
        if ground(x, y, z) == STONE || (wall && !door) {
            STONE
        } else {
            AIR
        }
    };
    let wall = move |x: i32, y: i32, z: i32| {
        if ground(x, y, z) == STONE || ((3..=5).contains(&x) && y < 80) {
            WOOL
        } else {
            AIR
        }
    };
    let cave = |x: i32, y: i32, z: i32| {
        let (dx, dy, dz) = (f64::from(x), f64::from(y - 20), f64::from(z));
        let chamber = dx * dx + dy * dy * 4.0 + dz * dz < 30.0 * 30.0;
        let tunnel = z.abs() <= 2 && (18..=23).contains(&y);
        if y < 100 && !chamber && !tunnel {
            STONE
        } else {
            AIR
        }
    };
    vec![
        Scene {
            name: "open ground",
            snapshot: Snapshot::new(world(ground), m.clone()),
            sound: [0.5, 64.5, 8.5],
            listener: [0.5, 65.6, 0.5],
        },
        Scene {
            name: "stone room, door",
            snapshot: Snapshot::new(world(room), m.clone()),
            sound: [0.5, 64.5, 0.5],
            listener: [3.5, 65.6, -14.5],
        },
        Scene {
            name: "behind 3 wool",
            snapshot: Snapshot::new(world(wall), m.clone()),
            sound: [0.5, 64.5, 0.5],
            listener: [10.5, 65.6, 0.5],
        },
        Scene {
            name: "large cave",
            snapshot: Snapshot::new(world(cave), m),
            sound: [4.5, 20.5, 6.5],
            listener: [-3.5, 21.6, 0.5],
        },
    ]
}
fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}
struct Discard;
impl Sink for Discard {
    fn environment(&self, _: ChannelId, _: Environment) {}
}

fn main() {
    let sounds: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(200);
    let scenes = scenes();
    println!("## Sound physics thread: one sound (ms, median / 95th percentile of {sounds})\n");
    let variants: [(&str, u32, u32); 5] = [
        ("8 rays x 4", 8, 4),
        ("16 rays x 4", 16, 4),
        ("32 rays x 4 (default)", 32, 4),
        ("32 rays x 8", 32, 8),
        ("64 rays x 4", 64, 4),
    ];
    print!("| scene |");
    for (name, _, _) in variants {
        print!(" {name} |");
    }
    println!("\n|---|{}", "---|".repeat(variants.len()));
    for scene in &scenes {
        print!("| {} |", scene.name);
        for (_, rays, bounces) in variants {
            let settings = Settings {
                enabled: true,
                environment_evaluation_ray_count: rays,
                environment_evaluation_ray_bounces: bounces,
                ..Settings::default()
            };
            let directions = sound_physics::ray_directions(rays);
            let mut times = Vec::with_capacity(sounds);
            for i in 0..sounds {
                let jitter = (i % 7) as f64 * 0.13;
                let request = Request {
                    at: [
                        scene.sound[0] + jitter,
                        scene.sound[1],
                        scene.sound[2] - jitter,
                    ],
                    listener: scene.listener,
                    listener_underwater: false,
                    source: 5,
                    event: "minecraft:entity.zombie.ambient".into(),
                };
                let started = Instant::now();
                black_box(sound_physics::evaluate(
                    &scene.snapshot,
                    &request,
                    16.0,
                    &settings,
                    &directions,
                ));
                times.push(started.elapsed());
            }
            times.sort();
            let at = |q: f64| ms(times[((times.len() - 1) as f64 * q) as usize]);
            print!(" {:.3} / {:.3} |", at(0.5), at(0.95));
        }
        println!();
    }

    println!("\n## Game thread\n");
    let mut columns: FxHashMap<(i32, i32), Vec<Arc<Section>>> = FxHashMap::default();
    for (&(cx, sy, cz), s) in &world(|_, y, _| if y < 64 { STONE } else { AIR }) {
        columns.entry((cx, cz)).or_insert_with(|| {
            (0..SECTIONS)
                .map(|_| Arc::new(Section::default()))
                .collect()
        })[sy as usize] = s.clone();
    }
    let mut acoustics = Acoustics::default();
    acoustics.set_settings(&Settings {
        enabled: true,
        ..Settings::default()
    });
    acoustics.start(Discard);
    let listener = [8.0, 70.0, 8.0];
    let started = Instant::now();
    acoustics.sync(listener, |x, z| columns.get(&(x, z)).map(|c| &c[..]));
    let first = started.elapsed();
    let ticks = 2000;
    let started = Instant::now();
    for _ in 0..ticks {
        acoustics.sync(listener, |x, z| columns.get(&(x, z)).map(|c| &c[..]));
    }
    let steady = started.elapsed() / ticks;
    // A block edit in every chunk each tick (the worst case: the sync resends them).
    let started = Instant::now();
    for _ in 0..200 {
        for c in columns.values_mut() {
            c[8] = Arc::new((*c[8]).clone());
        }
        acoustics.sync(listener, |x, z| columns.get(&(x, z)).map(|c| &c[..]));
    }
    let edits = started.elapsed() / 200;
    println!(
        "- section sync, {} sections: first {:.3} ms, unchanged tick {:.4} ms, one edit per chunk {:.4} ms (includes the copy-on-write of 81 sections)",
        acoustics.stats().sections,
        ms(first),
        ms(steady),
        ms(edits)
    );
    // The block facts' own names, as the game holds them.
    let facts: Vec<(Arc<str>, Arc<str>, Arc<str>)> = (0..30_000)
        .map(|i| {
            let sound = ["stone", "wool", "anvil", "crop", "glass"][i % 5];
            (
                format!("minecraft:block_{i}").into(),
                format!("minecraft:block.{sound}.break").into(),
                format!("minecraft:block.{sound}.step").into(),
            )
        })
        .collect();
    let started = Instant::now();
    let states: Vec<BlockState> = facts
        .iter()
        .map(|(block, break_sound, step_sound)| BlockState {
            block: block.clone(),
            break_sound: break_sound.clone(),
            step_sound: step_sound.clone(),
            fluid: FluidKind::None,
            collides: true,
            sturdy: 0b11_1111,
        })
        .collect();
    let handed = started.elapsed();
    let started = Instant::now();
    black_box(materials(&states, &Settings::default()));
    let built = started.elapsed();
    acoustics.set_states(states);
    println!(
        "- block states, 30 000 (once, when turned on): {:.2} ms to hand over; the thread builds the materials in {:.2} ms",
        ms(handed),
        ms(built)
    );
    println!("- starting a sound: one message to the thread (no tracing on the game thread)");

    println!("\n## Audio thread: mixing 1 s of 48 kHz stereo (ms)\n");
    println!("| voices | vanilla mixer | with sound physics |");
    println!("|---|---|---|");
    for voices in [8, 32, 64] {
        let off = horizon_client::audio::mix_benchmark(voices, false, 5.0) / 5;
        let on = horizon_client::audio::mix_benchmark(voices, true, 5.0) / 5;
        println!("| {voices} | {:.2} | {:.2} |", ms(off), ms(on));
    }
}
