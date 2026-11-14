//! The owner link: the closed set of requests the window sends its integrated server thread.
use super::{
    Dim, Game,
    integrated::{
        ServerOptions,
        capture::{Live, Target},
    },
    lan::LanSettings,
    lan_screen::Snapshot,
    memory::ServerSide,
};
use horizon_core::geometry::Vec3;
use horizon_perf_format::v1::StopReason;
use horizon_protocol::connection::Pipe;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{Receiver, Sender, SyncSender},
    },
};

/// Where the server puts the answer to a request the window waits for.
pub(super) type Reply<T> = SyncSender<T>;

/// Work the window hands its server thread between two ticks (`MinecraftServer.execute`).
pub(super) enum OwnerRequest {
    /// `IntegratedServer.publishServer`.
    Publish(LanSettings, Reply<Result<u16, String>>),
    /// `IntegratedServer.unpublishServer`: whether it was published.
    Unpublish(Reply<bool>),
    /// `horizon-ctl lan status`.
    LanStatus(Reply<serde_json::Value>),
    /// New command access and game mode forcing settings of the open world.
    LanApplySettings {
        guest_commands: bool,
        force: bool,
        done: Reply<()>,
    },
    /// The world options screen's state.
    ServerOptions(Reply<ServerOptions>),
    /// The world options screen's changes, from `before` to `wanted`.
    ApplyWorldOptions {
        before: Snapshot,
        wanted: Snapshot,
        done: Reply<()>,
    },
    /// Far terrain for a client in `dim`.
    LodSource(
        Dim,
        Reply<Option<Arc<dyn horizon_client::streaming::LodSource>>>,
    ),
    /// A splitscreen player (offline uuid) joins through memory.
    JoinSplit(u128, Reply<Option<Pipe>>),
    /// A splitscreen player (offline uuid) left.
    LeaveSplit(u128, Reply<()>),
    /// The pregen commands are on or off.
    SetPregen(bool),
    /// A live content mod switch: the data packs reload from the new root.
    ReloadMods(ModsReload),
    /// The server thread's side of a capture starts, for `target`; the reply, when one waits, gets its path.
    StartServerCapture(Target, Arc<Live>, Option<Reply<Result<PathBuf, String>>>),
    /// The server thread's capture stops; the saved path goes to the reply when one waits.
    StopServerCapture(StopReason, Option<Sender<String>>),
    /// `/profiler mark`: a named moment in the server thread's capture.
    MarkServerCapture(String),
    /// What agent control asks of the world.
    Control(ControlRequest),
}

/// The agent-control asks, answered on the server thread in the owner's level.
pub(super) enum ControlRequest {
    /// `simulation blocks`: the server's own blocks in a box.
    Blocks {
        lo: [i32; 3],
        hi: [i32; 3],
        reply: Reply<serde_json::Value>,
    },
    /// `simulation`: the server's clock and projectiles.
    Simulation(Reply<serde_json::Value>),
    /// `server-hold`: stop or resume the server's tick.
    HoldServer { held: bool, done: Reply<()> },
    /// A console command run as the owner; the chat lines it produced.
    ConsoleCommand {
        command: String,
        reply: Reply<Vec<String>>,
    },
    /// `time`: the server's day time.
    SetDayTime { time: i64, done: Reply<()> },
    /// `summon`: the new mob's id.
    Summon {
        kind: String,
        at: [f64; 3],
        baby: bool,
        reply: Reply<Result<u32, String>>,
    },
    /// `mobs`: the mobs near `center`.
    MobsAt {
        radius: f64,
        center: Vec3,
        reply: Reply<serde_json::Value>,
    },
    /// `hurtlog`.
    HurtLog(Reply<serde_json::Value>),
    /// `save` and `save-all`.
    Save {
        all: bool,
        reply: Reply<Result<(), String>>,
    },
    /// `setblock`, `fill`, `getblock` and `container`, with their words.
    BlockCommand {
        words: Vec<String>,
        reply: Reply<Result<Option<serde_json::Value>, String>>,
    },
    /// The memory report: the server's own parts.
    ServerSide(Reply<ServerSide>),
    /// `lan op NAME LEVEL`: whether a guest of that name was found.
    SetOp {
        name: String,
        level: u8,
        reply: Reply<bool>,
    },
    /// The save folder of the server's world.
    WorldRoot(Reply<PathBuf>),
    /// The recording's copy: save, then hold the server thread until `copied` is dropped.
    SaveThenWait {
        saved: Reply<Result<(), String>>,
        copied: Receiver<()>,
    },
}

/// What a live content mod switch changes on the server (`Game::mods_reloaded`).
pub(super) struct ModsReload {
    pub root: PathBuf,
    pub enabled: BTreeSet<String>,
}

impl Game<'_> {
    /// Answers one request of the window, on the server thread.
    pub(super) fn serve_owner_request(&mut self, request: OwnerRequest) {
        match request {
            OwnerRequest::Publish(settings, reply) => {
                let _ = reply.send(self.publish(settings));
            }
            OwnerRequest::Unpublish(reply) => {
                let _ = reply.send(self.unpublish());
            }
            OwnerRequest::LanStatus(reply) => {
                let _ = reply.send(self.lan_status());
            }
            OwnerRequest::LanApplySettings {
                guest_commands,
                force,
                done,
            } => {
                self.lan_apply_settings(guest_commands, force);
                let _ = done.send(());
            }
            OwnerRequest::ServerOptions(reply) => {
                let _ = reply.send(self.server_options());
            }
            OwnerRequest::ApplyWorldOptions {
                before,
                wanted,
                done,
            } => {
                self.apply_world_options(&before, &wanted);
                let _ = done.send(());
            }
            OwnerRequest::LodSource(dim, reply) => {
                let _ = reply.send(self.lod_source_in(dim));
            }
            OwnerRequest::JoinSplit(uuid, reply) => {
                let _ = reply.send(self.join_split_seat(uuid));
            }
            OwnerRequest::LeaveSplit(uuid, done) => {
                if let Some(host) = self.lan.as_mut() {
                    host.hub.ops.remove(&uuid);
                }
                let _ = done.send(());
            }
            OwnerRequest::SetPregen(enabled) => self.set_pregen_commands(enabled),
            OwnerRequest::ReloadMods(reload) => self.apply_mods_reload(reload),
            OwnerRequest::StartServerCapture(target, live, reply) => {
                let started = self.start_server_capture(target, live);
                match reply {
                    Some(reply) => {
                        let _ = reply.send(started);
                    }
                    None => {
                        if let Err(e) = started {
                            log::warn!("Horizon: the server thread is not captured: {e}");
                        }
                    }
                }
            }
            OwnerRequest::StopServerCapture(reason, reply) => {
                self.stop_server_capture(reason, reply)
            }
            OwnerRequest::MarkServerCapture(name) => self.mark_server_capture(&name),
            OwnerRequest::Control(request) => self.serve_control_request(request),
        }
    }

    /// Answers one agent-control ask, on the server thread.
    fn serve_control_request(&mut self, request: ControlRequest) {
        match request {
            ControlRequest::Blocks { lo, hi, reply } => {
                let _ = reply.send(self.in_owner_level(|g| g.blocks_json(lo, hi)));
            }
            ControlRequest::Simulation(reply) => {
                let _ = reply.send(self.in_owner_level(|g| g.simulation_status()));
            }
            ControlRequest::HoldServer { held, done } => {
                self.agent.hold_server = held;
                let _ = done.send(());
            }
            ControlRequest::ConsoleCommand { command, reply } => {
                let _ = reply.send(self.owner_console_command(&command));
            }
            ControlRequest::SetDayTime { time, done } => {
                self.world.day_time = time;
                let _ = done.send(());
            }
            ControlRequest::Summon {
                kind,
                at,
                baby,
                reply,
            } => {
                let id = self
                    .in_owner_level(|g| g.summon(&kind, Some(at), baby).map_err(|e| e.to_string()));
                let _ = reply.send(id);
            }
            ControlRequest::MobsAt {
                radius,
                center,
                reply,
            } => {
                let _ = reply.send(self.in_owner_level(|g| g.control_mobs_at(radius, center)));
            }
            ControlRequest::HurtLog(reply) => {
                let _ = reply.send(self.control_hurt_log());
            }
            ControlRequest::Save { all, reply } => {
                let _ = reply.send(self.control_save(all).map_err(|e| e.to_string()));
            }
            ControlRequest::BlockCommand { words, reply } => {
                let words: Vec<&str> = words.iter().map(String::as_str).collect();
                let out =
                    self.in_owner_level(|g| g.block_command(&words).map_err(|e| e.to_string()));
                let _ = reply.send(out);
            }
            ControlRequest::ServerSide(reply) => {
                let _ = reply.send(self.server_side());
            }
            ControlRequest::SetOp { name, level, reply } => {
                let _ = reply.send(self.lan_set_op(&name, level));
            }
            ControlRequest::WorldRoot(reply) => {
                let _ = reply.send(self.world_root.clone());
            }
            ControlRequest::SaveThenWait { saved, copied } => {
                let _ = saved.send(self.save_game().map_err(|e| e.to_string()));
                let _ = copied.recv();
            }
        }
    }

    /// A splitscreen seat gets the owner's permissions and a connection through memory.
    fn join_split_seat(&mut self, uuid: u128) -> Option<Pipe> {
        let host = self.lan.as_mut()?;
        let owner = host.owner.as_ref()?.uuid;
        let level = host.hub.ops.get(&owner).copied().unwrap_or(0);
        host.hub.ops.insert(uuid, level);
        host.hub.lan.connect_local().ok()
    }

    /// The command tree shows the pregen commands only while they are on.
    fn set_pregen_commands(&mut self, enabled: bool) {
        use horizon_client::command::vanilla;
        if vanilla::game_dispatcher().find("pregen").is_some() != enabled {
            vanilla::set_horizon_commands(enabled);
            self.resend_commands_filtered();
        }
    }
}
