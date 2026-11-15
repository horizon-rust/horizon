//! The anticheat on a dedicated server's hub (docs/anticheat.md), the `hub` feature: the checks run
//! through the engine's hook (`lan::watch`) where the hub handles what players send, and a flag is
//! carried out here (send back, alert, kick, or a ban the game adds to `banned-players.json`:
//! [`HostEvent::Ban`]). The rules and the violation levels are this crate's. A dedicated server has it on
//! by default; the hub holds none of this when `server.toml` turns it off (or for a LAN
//! world), and then does exactly what vanilla's server does.
use crate::{Check, Config, Player, Strikes};
use horizon_client_lib::collisions::SceneCollisions;
use horizon_client_lib::lan::guest::Guest;
use horizon_client_lib::lan::hub::{HostEvent, Hub, TickCtx};
use horizon_client_lib::lan::prediction::Over;
use horizon_client_lib::lan::watch::Watch;
use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::nbt::{JavaString, Tag};
use std::{io::Write, time::Instant};

pub struct Anticheat {
    pub config: Config,
    /// The thread writing `log`, started at the first flag: the tick only hands it the lines.
    log: Option<LogWriter>,
    /// The players' kicks by check, read from `strikes` at the first kick.
    strikes: Option<Strikes>,
    /// When the previous server tick started, for the timer's balance.
    last_tick: Option<Instant>,
    tick_rate: f32,
}

impl Anticheat {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            log: None,
            strikes: None,
            last_tick: None,
            tick_rate: 20.0,
        }
    }

    fn write_log(&mut self, line: &str) {
        if self.config.log.is_empty() {
            return;
        }
        if self.log.is_none() {
            let path = std::path::PathBuf::from(&self.config.log);
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let Ok(file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            else {
                return;
            };
            // A flood of flags (a lagging player, the load-test bots) wrote to the disk on the
            // tick thread, one line at a time: the lines go to a writer thread instead, in order.
            let (send, lines) = std::sync::mpsc::channel::<String>();
            let Ok(thread) = std::thread::Builder::new()
                .name("anticheat-log".into())
                .spawn(move || {
                    // The lines waiting are written together, flushed once none is left.
                    let mut file = std::io::BufWriter::new(file);
                    while let Ok(entry) = lines.recv() {
                        let _ = file.write_all(entry.as_bytes());
                        for entry in lines.try_iter() {
                            let _ = file.write_all(entry.as_bytes());
                        }
                        let _ = file.flush();
                    }
                })
            else {
                return;
            };
            self.log = Some(LogWriter {
                send: Some(send),
                thread: Some(thread),
            });
        }
        if let Some(send) = self.log.as_ref().and_then(|log| log.send.as_ref()) {
            let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = send.send(format!("[{time}] {line}\n"));
        }
    }

    /// Player `uuid` was kicked by `check`: its kicks by it so far and whether this one bans. The
    /// count is saved at once, so a restart doesn't forgive it.
    fn strike(&mut self, uuid: u128, check: Check) -> (u32, bool) {
        let path = std::path::Path::new(&self.config.strikes);
        let strikes = self.strikes.get_or_insert_with(|| {
            std::fs::read(path)
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                .unwrap_or_default()
        });
        let ban_after = self.config.actions(check).ban_after;
        let kick = strikes.kick(&uuid_text(uuid), check, ban_after);
        if !self.config.strikes.is_empty()
            && let Ok(json) = serde_json::to_vec_pretty(strikes)
        {
            let _ = std::fs::write(path, json);
        }
        kick
    }
}

/// The flag log's writer thread. Dropped with the anticheat (the server stopping), it closes the
/// channel and waits for the thread to write every line handed to it, so none is lost.
struct LogWriter {
    send: Option<std::sync::mpsc::Sender<String>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for LogWriter {
    fn drop(&mut self) {
        self.send = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A UUID as `banned-players.json` writes it.
fn uuid_text(uuid: u128) -> String {
    let h = format!("{uuid:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

/// Server ticks a claim of the ground keeps its height after the ground last held the player.
const GROUND_GRACE: u64 = 10;

/// Whether an entity of `kind` at `at` may carry a player standing at `player`: the entities players
/// stand on (boats, rafts, minecarts, shulkers, happy ghasts, 4 blocks tall), within 4 blocks sideways and
/// from a block above the player to a little more than a happy ghast's height below.
fn carries_player(kind: &str, at: [f64; 3], player: [f64; 3]) -> bool {
    let carrier = kind.ends_with("boat")
        || kind.ends_with("raft")
        || kind.contains("minecart")
        || kind == "minecraft:shulker"
        || kind == "minecraft:happy_ghast";
    let up = player[1] - at[1];
    carrier
        && (at[0] - player[0]).abs() < 4.0
        && (at[2] - player[2]).abs() < 4.0
        && (-1.0..=4.5).contains(&up)
}

fn text(s: &str, color: &str) -> Tag {
    Tag::compound([
        ("text", Tag::String(JavaString::from(s))),
        ("color", Tag::String(JavaString::from(color))),
    ])
}

/// Guest `g`'s violation levels and the checks' state, made at its first use.
fn player(g: &mut Guest) -> &mut Player {
    g.anticheat
        .get_or_insert_with(|| Box::new(Player::default()))
        .downcast_mut()
        .expect("the guest's anticheat state is the anticheat's Player")
}

/// The hub, while its hook runs the checks with the anticheat (`ac`) it lent.
struct On<'a>(&'a mut Hub);

impl std::ops::Deref for On<'_> {
    type Target = Hub;
    fn deref(&self) -> &Hub {
        self.0
    }
}

impl std::ops::DerefMut for On<'_> {
    fn deref_mut(&mut self) -> &mut Hub {
        self.0
    }
}

impl On<'_> {
    /// The start of a server tick: the time since the previous one fills each player's timer balance.
    fn anticheat_tick(&mut self, ac: &mut Anticheat, tick_rate: f32) {
        let now = Instant::now();
        let elapsed = ac.last_tick.replace(now).map(|t| now - t);
        if !ac.config.runs(Check::Timer) {
            return;
        }
        // A new tick rate reaches the client a little later: both start again.
        let restart = ac.tick_rate != tick_rate;
        ac.tick_rate = tick_rate;
        let ms = elapsed.map_or(0.0, |e| e.as_secs_f64() * 1000.0);
        for g in &mut self.guests {
            if restart {
                player(g).timer.reset();
            }
            player(g).timer.elapse(ms, &ac.config.timer);
        }
    }

    /// ServerboundClientTickEndPacket: one client tick spent.
    fn anticheat_tick_end(&mut self, ac: &mut Anticheat, i: usize, events: &mut Vec<HostEvent>) {
        if ac.config.runs(Check::Combat)
            && let Some(foul) = player(&mut self.guests[i]).attacks.tick_end()
        {
            self.combat_foul(ac, i, foul, events);
        }
        if !ac.config.runs(Check::Timer) {
            return;
        }
        let (rate, config) = (ac.tick_rate, &ac.config.timer);
        if let Some(ahead) = player(&mut self.guests[i]).timer.tick_end(rate, config) {
            self.flag(
                ac,
                i,
                Check::Timer,
                1.0,
                &format!("{ahead:.0} ms ahead"),
                events,
            );
        }
    }

    /// ServerboundAttackPacket on entity `target`: `combat`. Returns whether the attack is dropped:
    /// from `setback` on, every attack is, until the level decays below it.
    fn anticheat_attack(
        &mut self,
        ac: &mut Anticheat,
        i: usize,
        target: i32,
        events: &mut Vec<HostEvent>,
    ) -> bool {
        if !ac.config.runs(Check::Combat) {
            return false;
        }
        if let Some(foul) = player(&mut self.guests[i]).attacks.attack(target)
            && self.combat_foul(ac, i, foul, events)
        {
            return true;
        }
        let (setback, ticks) = (ac.config.combat.setback, self.ticks);
        setback > 0.0 && player(&mut self.guests[i]).vl(&ac.config, Check::Combat, ticks) >= setback
    }

    /// ServerboundPunchPacket: the arm swung, for `combat`.
    fn anticheat_swing(&mut self, i: usize) {
        player(&mut self.guests[i]).attacks.swing();
    }

    fn combat_foul(
        &mut self,
        ac: &mut Anticheat,
        i: usize,
        foul: crate::combat::Foul,
        events: &mut Vec<HostEvent>,
    ) -> bool {
        use crate::combat::Foul;
        let detail = match foul {
            Foul::NoSwing(n) => {
                format!("attacked without swinging ({n} more attacks than swings in a tick)")
            }
            Foul::TwoTargets(first, then) => {
                format!("attacked entities {first} and {then} in one tick")
            }
        };
        self.flag(ac, i, Check::Combat, 1.0, &detail, events)
    }

    /// `ground`, for a move from guest `i` that says it ends on the ground at `to` (`stands`: the move
    /// itself was stopped by something below): None when the claim stands (or isn't judged, or the check
    /// only logs: `setback = 0`), else whether the move is refused because the player was kicked. A
    /// false claim is dropped (vanilla's fall damage follows); the player is never sent back, and the
    /// move doesn't count toward vanilla's floating kick. A claim at the height where the
    /// ground last held the player, within half a second, isn't judged: the block may have gone
    /// server-side (broken, pushed) before the client heard.
    #[allow(clippy::too_many_arguments)]
    fn anticheat_ground(
        &mut self,
        ac: &mut Anticheat,
        i: usize,
        to: [f64; 3],
        stands: bool,
        ctx: &TickCtx,
        events: &mut Vec<HostEvent>,
    ) -> Option<bool> {
        if !ac.config.runs(Check::Ground) {
            return None;
        }
        // Standing still: no fall to skip, whatever holds the player up (a block the server doesn't
        // have, seen on the official server after a long teleport). Hovering is vanilla's floating kick.
        if self.guests[i].position == to {
            return None;
        }
        let ticks = self.ticks;
        // Entity.move resolves Y first, where the body starts, then X and Z: on the tick a player
        // walks off an edge (or lands on one and slides off it), the client stands on what is under
        // the move's start. A vanilla client did that once a step down every slope and staircase.
        let from = self.guests[i].position;
        let held_at_start = || self.ground_holds(i, [from[0], to[1], from[2]], ctx) != Some(false);
        if stands || self.ground_holds(i, to, ctx)? || held_at_start() {
            player(&mut self.guests[i]).ground = Some((ticks, to[1]));
            return None;
        }
        if player(&mut self.guests[i]).ground.is_some_and(|(t, y)| {
            ticks.saturating_sub(t) <= GROUND_GRACE && (y - to[1]).abs() < 1.0E-3
        }) {
            return None;
        }
        let (fx, fz) = (to[0].floor() as i32, to[2].floor() as i32);
        let below = if self
            .encoder
            .has_fluid(ctx.world.block(fx, to[1].floor() as i32 - 1, fz).0)
        {
            "on a fluid"
        } else {
            "in the air"
        };
        let detail = format!(
            "claims the ground {below} at {:.1} {:.1} {:.1}",
            to[0], to[1], to[2]
        );
        self.flag(ac, i, Check::Ground, 1.0, &detail, events);
        if ac.config.ground.setback == 0.0 {
            return None;
        }
        // The correction is the dropped claim, never a setback: sent back into the air it claimed to
        // stand in, an honest player (on a block the server doesn't have) claims it again and again,
        // and hovers there until vanilla's floating kick. Only the anticheat's own kick ends it.
        Some(!self.guests[i].left_reason.is_empty())
    }

    /// Whether something at `to` can hold guest `i` up; None: not judged. Only blocks that can't hold
    /// anyone are judged: nothing but air and fluids around the feet, and no collision a tenth of a block
    /// below. Anything else (powder snow, scaffolding, a block the server doesn't have yet) and a boat,
    /// minecart, shulker or happy ghast under the player: not judged.
    fn ground_holds(&self, i: usize, to: [f64; 3], ctx: &TickCtx) -> Option<bool> {
        let g = &self.guests[i];
        // No fall damage to skip.
        if g.creative() || g.spectator() || g.fall_flying {
            return None;
        }
        let binding = ctx.collision?;
        if !ctx.world.loaded(to[0].floor() as i32, to[2].floor() as i32) {
            return None;
        }
        let half = f64::from(g.dimensions().width) / 2.0;
        let lo =
            [to[0] - half - 0.0625, to[1] - 0.6, to[2] - half - 0.0625].map(|v| v.floor() as i32);
        let hi = [to[0] + half + 0.0625, to[1], to[2] + half + 0.0625].map(|v| v.floor() as i32);
        for x in lo[0]..=hi[0] {
            for z in lo[2]..=hi[2] {
                for y in lo[1]..=hi[1] {
                    let state = ctx.world.block(x, y, z).0;
                    if !self.encoder.is_air(state) && !self.encoder.has_fluid(state) {
                        return None;
                    }
                }
            }
        }
        if ctx
            .entities
            .iter()
            .any(|e| carries_player(&e.kind, e.position, to))
        {
            return None;
        }
        // A waterlogged slab or stairs: their shape holds the player.
        let height = f64::from(g.dimensions().height);
        let feet = Aabb::new(
            Vec3::new(to[0] - half, to[1], to[2] - half),
            Vec3::new(to[0] + half, to[1] + height, to[2] + half),
        );
        let mut scene = SceneCollisions {
            world: ctx.world,
            binding,
            subject: None,
        };
        let down = Vec3::new(0.0, -0.1, 0.0);
        Some(
            scene
                .resolve(feet, down, 0.0, false)
                .is_ok_and(|done| done.y > down.y),
        )
    }

    /// `check` failed for guest `i`: raise its level, log it, and act on it. Returns whether the move or
    /// action is cancelled (the player is sent back, or kicked).
    fn flag(
        &mut self,
        ac: &mut Anticheat,
        i: usize,
        check: Check,
        weight: f32,
        detail: &str,
        events: &mut Vec<HostEvent>,
    ) -> bool {
        self.flag_to(ac, i, check, weight, detail, events, None)
    }

    /// `prediction`: guest `conn`'s ticks that landed off the replay. A setback sends the player
    /// back to `agreed`, where its moves last agreed.
    fn anticheat_prediction(
        &mut self,
        ac: &mut Anticheat,
        conn: u32,
        over: &Over,
        agreed: [f64; 3],
    ) -> Vec<HostEvent> {
        let mut events = Vec::new();
        if let Some(i) = self.guests.iter().position(|g| g.conn == conn) {
            let ticks = if over.ticks == 1 { "tick" } else { "ticks" };
            // The worst tick as played, so the log tells a lag, a glide or a ladder from a cheat.
            let worst = over
                .at_worst
                .map_or(String::new(), |p| format!("; {}", p.describe()));
            let detail = format!(
                "moved up to {:.3} blocks off what its keys allow ({} {ticks}{worst})",
                over.worst, over.ticks
            );
            let weight = over.weight;
            self.flag_to(
                ac,
                i,
                Check::Prediction,
                weight,
                &detail,
                &mut events,
                Some(agreed),
            );
        }
        events
    }

    /// [`On::flag`], whose setback sends the player to `back` (where it was by default).
    #[allow(clippy::too_many_arguments)]
    fn flag_to(
        &mut self,
        ac: &mut Anticheat,
        i: usize,
        check: Check,
        weight: f32,
        detail: &str,
        events: &mut Vec<HostEvent>,
        back: Option<[f64; 3]>,
    ) -> bool {
        let ticks = self.ticks;
        // Kicked or banned already, and leaving: what it still had queued (a lagging client's ticks)
        // is dropped, and counts no second strike in the session the first kick warned.
        if !self.guests[i].left_reason.is_empty() {
            return true;
        }
        let g = &mut self.guests[i];
        let v = player(g).flag(&ac.config, check, weight, ticks);
        let (uuid, name) = (g.uuid, g.name.clone());
        let ban_after = ac.config.actions(check).ban_after;
        let (kicks, ban) = if v.kick {
            ac.strike(uuid, check)
        } else {
            (0, false)
        };
        let warned;
        let action = if ban {
            ", banned"
        } else if v.kick && ban_after > 0 {
            warned = format!(", kicked ({kicks} of {ban_after} before a ban)");
            &warned
        } else if v.kick {
            ", kicked"
        } else if v.setback && check == Check::Combat {
            ", attacks dropped"
        } else if v.setback && check == Check::Ground {
            ", claim dropped"
        } else if v.setback {
            ", sent back"
        } else {
            ""
        };
        let line = format!(
            "{name} failed {} (VL {:.1}): {detail}{action}",
            check.name(),
            v.vl
        );
        ac.write_log(&line);
        // A kick or a ban is always told, so an operator can undo one that was wrong.
        if v.alert || v.kick {
            events.push(HostEvent::Warn(format!("[Anticheat] {line}")));
            if ac.config.alerts {
                let level = ac.config.alert_level;
                let content = Tag::compound([
                    ("text", Tag::String(JavaString::from("[Anticheat] "))),
                    ("color", Tag::String(JavaString::from("red"))),
                    ("extra", Tag::List(vec![text(&line, "gray")])),
                ]);
                let packet = horizon_protocol::packets::play::Clientbound::SystemChat(
                    horizon_protocol::packets::play::SystemChat {
                        content,
                        overlay: false,
                    },
                );
                for op in self
                    .guests
                    .iter()
                    .filter(|o| o.placed && o.permission >= level)
                {
                    op.send(&packet);
                }
            }
        }
        if ban {
            // The game bans as `/ban` does, and its kick says so.
            events.push(HostEvent::Ban {
                uuid,
                name,
                reason: format!("Horizon anticheat: {}", check.name()),
            });
        } else if v.kick {
            let conn = self.guests[i].conn;
            self.anticheat_kick(conn);
        } else if v.setback
            && !matches!(check, Check::Combat | Check::Ground)
            && self.guests[i].awaiting_teleport.is_none()
        {
            // A combat setback drops the attack (`anticheat_attack`), a ground one the claim
            // (`anticheat_ground`); the others send the player back.
            self.resend_teleport(i, back);
        }
        v.setback || v.kick
    }
}

/// What [`load`] gives: the anticheat (`None`: off) and the lines to log, as (level, line).
#[derive(Default)]
pub struct Loaded {
    pub watch: Option<Box<dyn Watch>>,
    pub log: Vec<(&'static str, String)>,
}

/// The dedicated server's anticheat from `server.toml`'s `[anticheat]` (`None`: no file): the
/// default (on) without the file or the section, off (with an error logged) when the section is wrong.
pub fn load(server_toml: Option<&str>) -> Loaded {
    let mut loaded = Loaded::default();
    let config = match server_toml.map(Config::from_server_toml) {
        None => Config::default(),
        Some(Ok(config)) => {
            if config.enabled {
                loaded.log.push((
                    "INFO",
                    "Anticheat on (server.toml [anticheat]; enabled = false turns it off)".into(),
                ));
            }
            config
        }
        Some(Err(e)) => {
            loaded.log.push((
                "ERROR",
                format!("server.toml: {e}; the anticheat stays off"),
            ));
            return loaded;
        }
    };
    loaded.watch = watch(config);
    loaded
}

/// The hub's anticheat with these settings; none when they turn it off.
pub fn watch(config: Config) -> Option<Box<dyn Watch>> {
    config
        .enabled
        .then(|| Box::new(Anticheat::new(config)) as Box<dyn Watch>)
}

impl Watch for Anticheat {
    fn records(&self) -> bool {
        self.config.runs(Check::Prediction)
    }

    fn tolerance(&self) -> f64 {
        self.config.prediction.tolerance
    }

    fn tick(&mut self, hub: &mut Hub, tick_rate: f32) {
        On(hub).anticheat_tick(self, tick_rate);
    }

    fn tick_end(&mut self, hub: &mut Hub, i: usize, events: &mut Vec<HostEvent>) {
        On(hub).anticheat_tick_end(self, i, events);
    }

    fn attack(
        &mut self,
        hub: &mut Hub,
        i: usize,
        target: i32,
        events: &mut Vec<HostEvent>,
    ) -> bool {
        On(hub).anticheat_attack(self, i, target, events)
    }

    fn swing(&mut self, hub: &mut Hub, i: usize) {
        On(hub).anticheat_swing(i);
    }

    fn ground(
        &mut self,
        hub: &mut Hub,
        i: usize,
        to: [f64; 3],
        stands: bool,
        ctx: &TickCtx,
        events: &mut Vec<HostEvent>,
    ) -> Option<bool> {
        On(hub).anticheat_ground(self, i, to, stands, ctx, events)
    }

    fn prediction(
        &mut self,
        hub: &mut Hub,
        conn: u32,
        over: &Over,
        agreed: [f64; 3],
    ) -> Vec<HostEvent> {
        On(hub).anticheat_prediction(self, conn, over, agreed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The log's lines reach the file in the order flagged, every one of them by the time the
    /// anticheat is dropped (the server stopping), even a flood written just before.
    #[test]
    fn the_log_keeps_every_line_in_order_up_to_the_drop() {
        let dir =
            std::env::temp_dir().join(format!("horizon-anticheat-log-{}", std::process::id()));
        let path = dir.join("logs").join("anticheat.log");
        let mut anticheat = Anticheat::new(Config {
            log: path.to_string_lossy().into_owned(),
            ..Config::default()
        });
        for i in 0..20_000 {
            anticheat.write_log(&format!("flag {i}"));
        }
        drop(anticheat);
        let written = std::fs::read_to_string(&path).expect("the log was written");
        let _ = std::fs::remove_dir_all(&dir);
        let flags: Vec<&str> = written
            .lines()
            .map(|line| line.split_once("] ").expect("a timestamp").1)
            .collect();
        assert_eq!(flags.len(), 20_000);
        assert!(
            flags
                .iter()
                .enumerate()
                .all(|(i, f)| *f == format!("flag {i}"))
        );
    }

    /// Dropping the writer waits for its thread to write what it was handed, however far behind
    /// it is.
    #[test]
    fn dropping_the_writer_waits_for_its_lines() {
        let written = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let (send, lines) = std::sync::mpsc::channel::<String>();
        let out = std::sync::Arc::clone(&written);
        let thread = std::thread::spawn(move || {
            for line in lines {
                std::thread::sleep(std::time::Duration::from_millis(20));
                out.lock().unwrap().push(line);
            }
        });
        let writer = LogWriter {
            send: Some(send),
            thread: Some(thread),
        };
        for i in 0..5 {
            writer
                .send
                .as_ref()
                .unwrap()
                .send(format!("flag {i}"))
                .unwrap();
        }
        drop(writer);
        let expected: Vec<String> = (0..5).map(|i| format!("flag {i}")).collect();
        assert_eq!(*written.lock().unwrap(), expected);
    }

    #[test]
    fn uuids_are_written_as_the_ban_list_writes_them() {
        assert_eq!(
            uuid_text(0x069a79f4_44e9_4726_a5be_fca90e38aaf5),
            "069a79f4-44e9-4726-a5be-fca90e38aaf5"
        );
    }

    #[test]
    fn a_happy_ghasts_rider_and_a_boats_are_carried() {
        // HappyGhast is 4 blocks tall: its rider stands 4.0 above it.
        assert!(carries_player(
            "minecraft:happy_ghast",
            [0.0, 70.0, 0.0],
            [1.5, 74.0, -1.0]
        ));
        assert!(carries_player(
            "minecraft:oak_boat",
            [0.0, 62.0, 0.0],
            [0.0, 62.5625, 0.0]
        ));
        assert!(!carries_player(
            "minecraft:zombie",
            [0.0, 70.0, 0.0],
            [0.0, 71.0, 0.0]
        ));
        assert!(!carries_player(
            "minecraft:happy_ghast",
            [0.0, 70.0, 0.0],
            [0.0, 80.0, 0.0]
        ));
    }
}
