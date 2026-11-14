//! The order natural spawning walks its chunks in, for VanillaSpawnChunkOrderOracle: each level's
//! joins, moves and leaves go to the natural spawn chunk counter as the game gives them
//! (`SpawnChunkTracker::place` with the player's section, `retain`), and each spawning pass prints the
//! counter's size, its chunks in order, those chunks after `Util.shuffle` with a `LegacyRandom` of the
//! pass's seed, and that random's next `nextInt`.
use horizon_client::spawn_chunk_order::{SpawnChunkTracker, shuffle_spawning_chunks};
use horizon_core::random::{LegacyRandom, RandomSource};
use serde_json::Value;
use std::io::Write;

fn chunks(list: &[(i32, i32)]) -> String {
    let all: Vec<String> = list.iter().map(|(x, z)| format!("{x}:{z}")).collect();
    all.join(" ")
}

pub fn main() {
    let args: Vec<_> = super::args().collect();
    let input = std::fs::read_to_string(&args[1]).unwrap();
    let mut out = std::io::BufWriter::new(std::fs::File::create(&args[2]).unwrap());
    for (index, line) in input.lines().enumerate() {
        let case: Value = serde_json::from_str(line).unwrap();
        let mut tracker = SpawnChunkTracker::default();
        writeln!(out, "== {index}").unwrap();
        for op in case["ops"].as_array().unwrap() {
            let int = |i: usize| op[i].as_i64().unwrap();
            match op[0].as_str().unwrap() {
                "join" | "move" => {
                    let section = [int(2) as i32, int(3) as i32, int(4) as i32];
                    tracker.place(int(1) as u128, section);
                }
                "leave" => tracker.retain(|id| id != int(1) as u128),
                "pass" => {
                    tracker.run_updates();
                    let mut order: Vec<_> = tracker.chunks().collect();
                    writeln!(out, "{} | {}", tracker.len(), chunks(&order)).unwrap();
                    let mut random = LegacyRandom::new(int(1));
                    shuffle_spawning_chunks(&mut order, &mut random);
                    writeln!(out, "{} | {}", random.next_int(), chunks(&order)).unwrap();
                }
                other => panic!("unknown op {other}"),
            }
        }
    }
}
