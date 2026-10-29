//! What `horizon-server` asks the dedicated server on its control endpoint (`--control-socket`, docs/server.md
//! section 2): `server-status` answers with the tick rate, the tick times `/tick query` reports, the players
//! and the loaded chunks; `console <line>` runs a console command and answers with the lines it logged.
use super::{ServerState, capture_log};
use crate::game::Game;
use serde_json::{Value, json};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

/// The window the tick rate is measured over.
const TPS_WINDOW: Duration = Duration::from_secs(5);

fn console_response(command: impl FnOnce()) -> Value {
    let capture = capture_log(command);
    let output: Vec<String> = capture
        .lines
        .into_iter()
        .map(|line| {
            line.strip_prefix("System chat: ")
                .map_or(line.clone(), str::to_owned)
        })
        .collect();
    let mut response = json!({ "output": output });
    if let Some(error) = capture.error {
        response["error"] = json!(error);
    }
    response
}

impl ServerState {
    /// One pass of the server loop ended.
    pub(super) fn ticked(&mut self, at: Instant) {
        self.ticks.push_back(at);
        while self
            .ticks
            .front()
            .is_some_and(|t| at.duration_since(*t) > TPS_WINDOW)
        {
            self.ticks.pop_front();
        }
    }
    /// Ticks per second over the last few seconds.
    fn tps(&self) -> f64 {
        match (self.ticks.front(), self.ticks.back()) {
            (Some(first), Some(last)) if self.ticks.len() > 1 => {
                (self.ticks.len() - 1) as f64 / last.duration_since(*first).as_secs_f64().max(1e-9)
            }
            _ => 0.0,
        }
    }
}

impl Game<'_> {
    /// Answers the requests waiting on the control endpoint.
    pub(super) fn poll_server_control(&mut self, state: &mut ServerState) {
        while let Some(request) = self.control.as_ref().and_then(|c| c.try_recv()) {
            let reply = match request.line.trim().split_once(' ') {
                _ if request.line.trim() == "server-status" => self.server_status(state),
                Some(("backup-begin", id)) => self.begin_backup(id),
                Some(("backup-end", id)) => {
                    let result = (|| -> horizon_client::Result<()> {
                        self.world_root
                            .persistence()
                            .ok_or("no active world")?
                            .end(id)
                    })();
                    match result {
                        Ok(()) => {
                            if state.stop_pending
                                && self
                                    .world_root
                                    .persistence()
                                    .unwrap()
                                    .held()
                                    .is_ok_and(|id| id.is_none())
                            {
                                state.stop = true;
                            }
                            json!({"ok": true, "snapshot": id})
                        }
                        Err(e) => json!({"error": e.to_string()}),
                    }
                }
                Some(("console", line)) => {
                    // Command feedback is logged as system chat; the reply carries the text alone.
                    console_response(|| self.console_command(state, line))
                }
                _ => json!({ "error": format!("unknown request {:?}", request.line) }),
            };
            let _ = request.reply.send(reply.to_string());
        }
    }

    fn begin_backup(&mut self, id: &str) -> Value {
        let result = (|| -> horizon_client::Result<()> {
            let owner = self
                .world_root
                .persistence()
                .ok_or("no active world")?
                .clone();
            if owner.can_begin(id)? {
                return owner.begin(id);
            }
            self.save_game()?;
            owner.begin(id)
        })();
        match result {
            Ok(()) => {
                json!({"ok": true, "snapshot": id, "world": self.world_root.file_name().unwrap_or_default().to_string_lossy()})
            }
            Err(e) => json!({"error": e.to_string()}),
        }
    }

    fn server_status(&self, state: &ServerState) -> Value {
        let millis = |nanos: u64| nanos as f64 / 1e6;
        let samples = self.clock.sorted_samples();
        // The percentiles `/tick query` prints (TickCommand.percentileToString).
        let at = |q: f64| {
            let i = ((samples.len() as f64 * q) as usize).min(samples.len().saturating_sub(1));
            samples.get(i).copied().map_or(0.0, millis)
        };
        let (players, max_players) = self.lan.as_ref().map_or((Vec::new(), 0), |h| {
            let players = h
                .hub
                .guests
                .iter()
                .map(|g| {
                    json!({
                        "name": g.name,
                        "uuid": horizon_protocol::crypto::uuid_string(g.uuid),
                        "address": g.address.ip().to_string(),
                        "ping_ms": g.latency.load(Ordering::Relaxed),
                    })
                })
                .collect();
            (players, h.hub.lan.shared.max_players())
        });
        json!({
            "tps": state.tps(),
            "target_tps": self.clock.rate(),
            "mspt": {
                "average": millis(self.clock.average_nanos()),
                "p50": at(0.5),
                "p95": at(0.95),
                "p99": at(0.99),
            },
            "players": players,
            "max_players": max_players,
            "chunks": self.world.chunks.len(),
            "paused": state.paused,
            "snapshot": self.world_root.persistence().and_then(|p| p.held().ok().flatten()),
            "autosave": self.world_root.persistence().is_none_or(|p| p.autosave_enabled()),
            "stop_pending": state.stop_pending,
            "persistence_error": self.world_root.persistence().and_then(|p| p.check().err()).map(|e| e.to_string()),
        })
    }
}
