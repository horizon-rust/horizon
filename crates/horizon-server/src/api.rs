//! The control API (docs/server.md, section 8): HTTP and JSON on 127.0.0.1, with a random token in
//! `.horizon/api.json` that only the server's own user can read. The CLI manages a running server through
//! it; the web panel and the MCP server will too.
//!
//! Its routes are [`ROUTES`], listed in docs/api/server-reference.md.

use crate::{
    backup,
    config::Config,
    engine,
    http::{self, Request, Response},
    panel,
    update::{self, Install},
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::Write,
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const ENDPOINT: &str = ".horizon/api.json";
/// The log lines kept for `logs` and the panel's console.
const LOG_LINES: usize = 2000;
/// The samples kept for the panel's graphs: 10 minutes, one a second.
const METRICS: usize = 600;

/// One endpoint of the control API: what the reference (docs/api/server-reference.md) says about it.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "only the reference reads all of it")
)]
pub struct Route {
    pub method: &'static str,
    pub path: &'static str,
    /// The JSON body or query it takes, "" for none.
    pub takes: &'static str,
    /// The JSON it answers with.
    pub answers: &'static str,
    pub what: &'static str,
}

/// Every endpoint `start` and [`route`] answer; a test checks the two agree.
pub const ROUTES: [Route; 11] = [
    Route {
        method: "GET",
        path: "/api/v1/status",
        takes: "",
        answers: "`phase`, `version`, `pid`, `uptime_seconds`, `engine_pid`, `engine_uptime_seconds`, `world`, `crashes`, `last_crash`, `sample` (TPS, MSPT, players, RAM, CPU), `update`, `restore`",
        what: "How the server does",
    },
    Route {
        method: "GET",
        path: "/api/v1/metrics",
        takes: "`?since=T` (seconds since 1970)",
        answers: "`{\"samples\": [...]}`: one a second, the last 10 minutes",
        what: "The samples taken after T, for graphs",
    },
    Route {
        method: "GET",
        path: "/api/v1/logs",
        takes: "`?since=N`",
        answers: "`{\"next\": N, \"lines\": [...]}`",
        what: "The log lines after the Nth (the last 2000 are kept)",
    },
    Route {
        method: "POST",
        path: "/api/v1/command",
        takes: "`{\"line\": \"<one command>\"}`",
        answers: "the lines the command logged",
        what: "Run a console command on the running game",
    },
    Route {
        method: "POST",
        path: "/api/v1/stop",
        takes: "",
        answers: "`{\"ok\": true}`",
        what: "Stop the game and the supervisor",
    },
    Route {
        method: "POST",
        path: "/api/v1/restart",
        takes: "",
        answers: "`{\"ok\": true}`",
        what: "Restart the game",
    },
    Route {
        method: "GET",
        path: "/api/v1/backups",
        takes: "",
        answers: "`{\"backups\": [{\"name\", \"bytes\"}]}`",
        what: "List the backups",
    },
    Route {
        method: "POST",
        path: "/api/v1/backups",
        takes: "`{\"name\": \"...\"}` (optional)",
        answers: "`{\"name\": \"<file>\"}`",
        what: "Back up now (a running game saves first and stays up)",
    },
    Route {
        method: "POST",
        path: "/api/v1/restore",
        takes: "`{\"name\": \"<backup>\"}`",
        answers: "`{\"name\": \"<backup>\", \"id\": N}`; `status.restore.last` reports `id`, `name`, `world`, `engine_pid`, `ok`, `message` once the replacement runs or the restore fails",
        what: "Restart the game with that backup's world (the current one is kept in backups/)",
    },
    Route {
        method: "POST",
        path: "/api/v1/update",
        takes: "`{\"tag\": \"...\", \"timeout\": seconds}` (both optional)",
        answers: "`{\"ok\": true}`; `status` says how it went (`update`)",
        what: "Restart on that installed release, or download the newest first; goes back if the game isn't up within the timeout",
    },
    Route {
        method: "POST",
        path: "/api/v1/panel/login",
        takes: "",
        answers: "`{\"url\": \"...\"}`",
        what: "A one-time login link for the web panel (with the token only, not from the panel)",
    },
];

/// What the supervisor is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Starting,
    Running,
    Stopping,
    /// Stopping the game to start it again (`restart`), or waiting to after a crash.
    Restarting,
}

impl Phase {
    fn name(self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Restarting => "restarting",
        }
    }
}

pub struct State {
    pub phase: Phase,
    pub world_name: Option<String>,
    pub engine_pid: Option<u32>,
    pub engine_started: Option<Instant>,
    pub crashes: u32,
    pub last_crash: Option<String>,
    /// The latest sample (`sample`): the game's status, RAM and CPU.
    pub sample: Value,
}

/// The log, numbered from the first line since the supervisor started.
#[derive(Default)]
pub struct Log {
    lines: VecDeque<String>,
    first: u64,
}

impl Log {
    pub fn push(&mut self, line: String) {
        if self.lines.len() == LOG_LINES {
            self.lines.pop_front();
            self.first += 1;
        }
        self.lines.push_back(line);
    }
    /// The lines after the `since`th, and the number to ask from next.
    pub fn since(&self, since: u64) -> (u64, Vec<String>) {
        let skip = since.saturating_sub(self.first) as usize;
        let next = self.first + self.lines.len() as u64;
        (next, self.lines.iter().skip(skip).cloned().collect())
    }
}

/// How long a game started on another release has to come up before the switch is undone.
pub const UPDATE_TIMEOUT: Duration = Duration::from_secs(300);

/// Updates and rollbacks (update.rs): what the check found, what is under way, how the last one ended.
#[derive(Default)]
pub struct Updates {
    /// A release newer than the one in use, from the last check.
    pub available: Option<String>,
    /// What is happening now, or nothing.
    pub busy: Option<String>,
    /// How the last update or rollback ended, and whether it went well.
    pub last: Option<String>,
    pub ok: bool,
    /// Counts the updates that ended, so a client can wait for the one it asked for.
    pub finished: u64,
    /// The release to restart the game on, and how long it has to come up.
    pub pending: Option<(String, Duration)>,
}

#[derive(Clone, Debug)]
pub struct RestoreRequest {
    pub id: u64,
    pub name: String,
    pub world: String,
}

#[derive(Clone, Debug)]
pub struct RestoreResult {
    pub id: u64,
    pub name: String,
    pub world: String,
    pub engine_pid: Option<u32>,
    pub ok: bool,
    pub message: String,
}

#[derive(Default)]
pub struct RestoreState {
    pub pending: Option<RestoreRequest>,
    pub requested: u64,
    pub finished: u64,
    pub last: Option<RestoreResult>,
}

pub enum RestoreError {
    NotFound(String),
    Conflict(String),
}

/// What the supervisor and the API threads share.
pub struct Shared {
    pub dir: PathBuf,
    pub state: Mutex<State>,
    pub log: Mutex<Log>,
    pub stop: AtomicBool,
    pub restart: AtomicBool,
    /// The pending restore and the last completed restore.
    pub restore: Mutex<RestoreState>,
    /// Serializes backups, console commands and engine lifecycle changes.
    pub lifecycle: Mutex<()>,
    pub started: Instant,
    /// The last samples, oldest first, for the panel's graphs.
    pub metrics: Mutex<VecDeque<Value>>,
    /// The panel's address, login links and sessions.
    pub panel: Mutex<panel::Auth>,
    /// The installer's folder this program runs from (none for a build of your own: no updates).
    pub install: Option<Install>,
    pub update: Mutex<Updates>,
}

impl Shared {
    pub fn new(dir: &Path) -> Arc<Self> {
        Self::with_install(dir, Install::find())
    }

    pub fn with_install(dir: &Path, install: Option<Install>) -> Arc<Self> {
        Arc::new(Self {
            dir: dir.to_owned(),
            state: Mutex::new(State {
                phase: Phase::Starting,
                world_name: None,
                engine_pid: None,
                engine_started: None,
                crashes: 0,
                last_crash: None,
                sample: Value::Null,
            }),
            log: Mutex::new(Log::default()),
            stop: AtomicBool::new(false),
            restart: AtomicBool::new(false),
            restore: Mutex::new(RestoreState::default()),
            lifecycle: Mutex::new(()),
            started: Instant::now(),
            metrics: Mutex::new(VecDeque::new()),
            panel: Mutex::new(panel::Auth::default()),
            install,
            update: Mutex::new(Updates::default()),
        })
    }

    /// Ends an update or rollback: its outcome, in the log and in `status`.
    pub fn finish_update(&self, ok: bool, message: String) {
        {
            let mut update = self.update.lock().unwrap();
            update.busy = None;
            update.ok = ok;
            update.finished += 1;
            if ok {
                update.available = None;
            }
            update.last = Some(message.clone());
        }
        crate::supervisor::note(self, if ok { "INFO" } else { "ERROR" }, &message);
    }

    /// Asks for a restart on the installed release `tag` (the supervisor switches when the game stops).
    fn switch_to(&self, tag: String, timeout: Duration) {
        self.update.lock().unwrap().busy = Some(format!("Restarting the game on {tag}"));
        self.update.lock().unwrap().pending = Some((tag, timeout));
        self.restart.store(true, Ordering::SeqCst);
    }

    /// `POST /api/v1/update`: switches to `tag`, or downloads the newest release first.
    fn start_update(
        self: &Arc<Self>,
        tag: Option<String>,
        timeout: Duration,
    ) -> Result<(), String> {
        let install = self.install.clone().ok_or(
            "this horizon-server wasn't installed by install.sh or install.ps1: no updates",
        )?;
        {
            let mut update = self.update.lock().unwrap();
            if let Some(busy) = &update.busy {
                return Err(format!("an update is under way: {busy}"));
            }
            update.busy = Some("Looking for the newest release".into());
        }
        if let Some(tag) = tag {
            if !install.release(&tag).is_dir() || tag.contains(['/', '\\']) {
                self.update.lock().unwrap().busy = None;
                return Err(format!(
                    "{tag} isn't installed (`horizon-server update` downloads it)"
                ));
            }
            self.switch_to(tag, timeout);
            return Ok(());
        }
        let shared = self.clone();
        std::thread::Builder::new()
            .name("update".into())
            .spawn(move || {
                let fetched = (|| {
                    let current = install.current().unwrap_or_default();
                    let release = update::release(None)?;
                    if !update::newer(&release.tag, &current) {
                        return Ok(None);
                    }
                    shared.update.lock().unwrap().busy =
                        Some(format!("Downloading {} ({})", release.tag, release.asset));
                    let archive = update::download(&install, &release)?;
                    let unpacked = update::unpack(&install, &archive, &release.tag);
                    let _ = std::fs::remove_file(&archive);
                    unpacked.map(|_| Some(release.tag))
                })();
                match fetched {
                    Ok(Some(tag)) => shared.switch_to(tag, timeout),
                    Ok(None) => shared.finish_update(true, "Already on the newest release".into()),
                    Err(e) => shared.finish_update(false, format!("Not updated: {e}")),
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    /// Keeps one sample for the graphs.
    pub fn record(&self, sample: Value) {
        let mut metrics = self.metrics.lock().unwrap();
        if metrics.len() == METRICS {
            metrics.pop_front();
        }
        metrics.push_back(sample);
    }
    pub fn set_phase(&self, phase: Phase) {
        self.state.lock().unwrap().phase = phase;
    }

    /// Accepts a restore while protecting selection from automatic retention.
    pub fn request_restore(&self, requested: &str) -> Result<RestoreRequest, RestoreError> {
        let _operation = self.lifecycle.lock().unwrap();
        let (phase, active_world) = {
            let state = self.state.lock().unwrap();
            (state.phase, state.world_name.clone())
        };
        if phase != Phase::Running {
            return Err(RestoreError::Conflict(
                "the game isn't running right now".into(),
            ));
        }
        let active_world = active_world
            .ok_or_else(|| RestoreError::Conflict("the running world is unknown".into()))?;
        let config = Config::load(&self.dir).map_err(RestoreError::Conflict)?;
        if config.world.name != active_world {
            return Err(RestoreError::Conflict(format!(
                "world.name changed from {active_world:?} to {:?}; restore must wait until the server restarts",
                config.world.name
            )));
        }
        let name = backup::find(&self.dir, requested).map_err(RestoreError::NotFound)?;
        backup::check_world(&self.dir, &name, &active_world).map_err(RestoreError::Conflict)?;

        let mut restore = self.restore.lock().unwrap();
        if restore.requested > restore.finished {
            return Err(RestoreError::Conflict(
                "a restore is already under way".into(),
            ));
        }
        let id = restore
            .requested
            .checked_add(1)
            .ok_or_else(|| RestoreError::Conflict("too many restore requests".into()))?;
        let request = RestoreRequest {
            id,
            name,
            world: active_world,
        };
        restore.requested = id;
        restore.pending = Some(request.clone());
        self.restart.store(true, Ordering::SeqCst);
        Ok(request)
    }

    /// Takes the accepted restore. The supervisor calls this while holding `lifecycle` through use.
    pub fn take_restore(&self) -> Option<RestoreRequest> {
        self.restore.lock().unwrap().pending.take()
    }

    /// Records the completed restore for API callers waiting through the restart.
    pub fn finish_restore(&self, result: RestoreResult) {
        let mut restore = self.restore.lock().unwrap();
        if result.id == restore.requested && result.id > restore.finished {
            restore.finished = result.id;
            restore.last = Some(result);
        }
    }

    /// Applies automatic retention without deleting a restore accepted by the API.
    pub fn prune_backups(&self, keep: usize) -> Vec<String> {
        let _operation = self.lifecycle.lock().unwrap();
        let protected = self
            .restore
            .lock()
            .unwrap()
            .pending
            .as_ref()
            .map(|request| request.name.clone());
        backup::prune_protecting(&self.dir, keep, protected.as_deref())
    }
    /// Archive the running world's held snapshot, then release it.
    pub fn back_up(&self, file_name: &str) -> Result<PathBuf, String> {
        let _operation = self.lifecycle.lock().unwrap();
        let mut config = Config::load(&self.dir)?;
        config.world.name = {
            let state = self.state.lock().unwrap();
            if state.phase != Phase::Running {
                return Err("the game isn't running: try again after it starts".into());
            }
            state
                .world_name
                .clone()
                .ok_or("the running world is unknown")?
        };
        let mut game = engine::Control::connect(&self.dir)
            .map_err(|e| format!("the game didn't answer: {e}"))?;
        let snapshot = random_token();
        let ask = |game: &mut engine::Control, command: &str| -> Result<Value, String> {
            let reply = game.ask(command).map_err(|e| e.to_string())?;
            if let Some(error) = reply["error"].as_str() {
                return Err(error.to_owned());
            }
            if reply["ok"] != true || reply["snapshot"].as_str() != Some(snapshot.as_str()) {
                return Err("the game did not acknowledge this backup snapshot".into());
            }
            Ok(reply)
        };
        let result = (|| {
            let held = ask(&mut game, &format!("backup-begin {snapshot}"))
                .map_err(|e| format!("the game didn't save: {e}"))?;
            if held["world"].as_str() != Some(config.world.name.as_str()) {
                return Err("the game acknowledged a different world".into());
            }
            backup::create(&self.dir, &config, file_name)
        })();
        let release = format!("backup-end {snapshot}");
        let resume = ask(&mut game, &release).or_else(|_| {
            let mut game = engine::Control::connect(&self.dir).map_err(|e| e.to_string())?;
            ask(&mut game, &release)
        });
        if let Err(error) = resume {
            let made = match result {
                Ok(path) => format!("backed up to {}", path.display()),
                Err(error) => error,
            };
            return Err(format!("{made}; the game didn't resume saving: {error}"));
        }
        result
    }
}

fn status(shared: &Shared) -> Value {
    let state = shared.state.lock().unwrap();
    let restore = shared.restore.lock().unwrap();
    json!({
        "phase": state.phase.name(),
        "version": crate::VERSION,
        "pid": std::process::id(),
        "uptime_seconds": shared.started.elapsed().as_secs(),
        "engine_pid": state.engine_pid,
        "world": state.world_name,
        "engine_uptime_seconds": state.engine_started.map(|t| t.elapsed().as_secs()),
        "crashes": state.crashes,
        "last_crash": state.last_crash,
        "sample": state.sample,
        "update": updates(shared),
        "restore": {
            "requested": restore.requested,
            "finished": restore.finished,
            "busy": restore.requested > restore.finished,
            "last": restore.last.as_ref().map(|last| json!({
                "id": last.id,
                "name": last.name,
                "world": last.world,
                "engine_pid": last.engine_pid,
                "ok": last.ok,
                "message": last.message,
            })),
        },
    })
}

fn updates(shared: &Shared) -> Value {
    let Some(install) = &shared.install else {
        return Value::Null;
    };
    let update = shared.update.lock().unwrap();
    json!({
        "installed": install.current(),
        "previous": install.previous(),
        "available": update.available,
        "busy": update.busy,
        "last": update.last,
        "ok": update.ok,
        "finished": update.finished,
    })
}

/// A random token: two 64-bit hashes keyed by the OS's random hasher seeds, the time and the process.
pub fn random_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    (0..2u64)
        .map(|i| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(i);
            h.write_u128(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos()),
            );
            h.write_u32(std::process::id());
            format!("{:016x}", h.finish())
        })
        .collect()
}

pub fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// Writes a file only its owner can read.
pub fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(text.as_bytes())
}

/// Removes the endpoint file when the supervisor ends.
pub struct Endpoint(PathBuf);
impl Drop for Endpoint {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Starts the API on a free port of 127.0.0.1 and writes where it is and its token.
pub fn start(shared: Arc<Shared>) -> Result<Endpoint, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("control API: {e}"))?;
    let address = listener.local_addr().map_err(|e| e.to_string())?;
    let token = random_token();
    let path = shared.dir.join(ENDPOINT);
    write_private(
        &path,
        &json!({ "address": address.to_string(), "token": token, "pid": std::process::id() })
            .to_string(),
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    let hosts = [address.to_string(), format!("localhost:{}", address.port())];
    http::serve(listener, move |request| {
        // A page in a browser can't send the token, and a rebound DNS name can't pass as this address.
        let authorised = request
            .header("authorization")
            .and_then(|h| h.strip_prefix("Bearer "))
            .is_some_and(|t| same_token(t.trim(), &token));
        if !authorised {
            return Response::error(401, "missing or wrong token (see .horizon/api.json)");
        }
        if !request
            .header("host")
            .is_some_and(|h| hosts.iter().any(|a| a == h))
        {
            return Response::error(403, "wrong Host");
        }
        if (request.method.as_str(), request.path.as_str()) == ("POST", "/api/v1/panel/login") {
            return match shared.panel.lock().unwrap().login_link() {
                Some(url) => Response::json(200, &json!({ "url": url })),
                None => Response::error(
                    409,
                    "the panel is off: set [panel] bind in server.toml and restart",
                ),
            };
        }
        route(&shared, request)
    })
    .map_err(|e| e.to_string())?;
    Ok(Endpoint(path))
}

/// The API's routes, for the CLI (with the token) and the panel (with a session).
pub fn route(shared: &Arc<Shared>, request: &Request) -> Response {
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/api/v1/status") => Response::json(200, &status(shared)),
        ("GET", "/api/v1/metrics") => {
            let since: u64 = request
                .param("since")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let samples: Vec<Value> = shared
                .metrics
                .lock()
                .unwrap()
                .iter()
                .filter(|m| m["t"].as_u64().is_some_and(|t| t > since))
                .cloned()
                .collect();
            Response::json(200, &json!({ "samples": samples }))
        }
        ("GET", "/api/v1/logs") => {
            let since = request
                .param("since")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let (next, lines) = shared.log.lock().unwrap().since(since);
            Response::json(200, &json!({ "next": next, "lines": lines }))
        }
        ("POST", "/api/v1/command") => {
            let line = serde_json::from_slice::<Value>(&request.body)
                .ok()
                .and_then(|v| v["line"].as_str().map(str::to_owned));
            let Some(line) = line.filter(|l| !l.trim().is_empty() && !l.contains('\n')) else {
                return Response::error(400, "the body must be {\"line\": \"<one command>\"}");
            };
            let _operation = shared.lifecycle.lock().unwrap();
            if shared.state.lock().unwrap().phase != Phase::Running {
                return Response::error(409, "the game isn't running right now");
            }
            if line
                .trim()
                .trim_start_matches('/')
                .split_whitespace()
                .next()
                == Some("stop")
            {
                shared.set_phase(Phase::Stopping);
                shared.stop.store(true, Ordering::SeqCst);
            }
            match engine::Control::connect(&shared.dir)
                .and_then(|mut c| c.ask(&format!("console {}", line.trim_start_matches('/'))))
            {
                Ok(reply) => Response::json(200, &reply),
                Err(e) => Response::error(409, &format!("the game didn't answer: {e}")),
            }
        }
        ("POST", "/api/v1/stop") => {
            shared.stop.store(true, Ordering::SeqCst);
            Response::json(200, &json!({ "ok": true }))
        }
        ("POST", "/api/v1/restart") => {
            shared.restart.store(true, Ordering::SeqCst);
            Response::json(200, &json!({ "ok": true }))
        }
        ("GET", "/api/v1/backups") => {
            let list: Vec<Value> = backup::list(&shared.dir)
                .into_iter()
                .map(|b| json!({ "name": b.name, "bytes": b.bytes }))
                .collect();
            Response::json(200, &json!({ "backups": list }))
        }
        ("POST", "/api/v1/backups") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let made = backup::file_name(std::time::SystemTime::now(), body["name"].as_str())
                .and_then(|name| shared.back_up(&name));
            match made {
                Ok(path) => Response::json(
                    200,
                    &json!({ "name": path.file_name().map(|n| n.to_string_lossy()) }),
                ),
                Err(e) => Response::error(409, &e),
            }
        }
        ("POST", "/api/v1/restore") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            match shared.request_restore(body["name"].as_str().unwrap_or_default()) {
                Ok(restore) => {
                    Response::json(200, &json!({ "name": restore.name, "id": restore.id }))
                }
                Err(RestoreError::NotFound(e)) => Response::error(404, &e),
                Err(RestoreError::Conflict(e)) => Response::error(409, &e),
            }
        }
        ("POST", "/api/v1/update") => {
            let body: Value = serde_json::from_slice(&request.body).unwrap_or(Value::Null);
            let timeout = body["timeout"]
                .as_u64()
                .map_or(UPDATE_TIMEOUT, Duration::from_secs);
            match shared.start_update(body["tag"].as_str().map(str::to_owned), timeout) {
                Ok(()) => Response::json(200, &json!({ "ok": true })),
                Err(e) => Response::error(409, &e),
            }
        }
        (_, path) if ROUTES.iter().any(|r| r.path == path) => Response::error(405, "wrong method"),
        _ => Response::error(404, "no such endpoint"),
    }
}

/// The API of the server running in a folder, for the CLI.
pub struct Client {
    address: SocketAddr,
    token: String,
}

impl Client {
    /// The running server's API; `None` when no server runs in `dir`.
    pub fn open(dir: &Path) -> Result<Option<Self>, String> {
        if crate::supervisor::lock(dir).is_ok() {
            return Ok(None);
        }
        // The supervisor writes the endpoint just after taking the lock: give it a moment.
        let path = dir.join(ENDPOINT);
        let deadline = Instant::now() + Duration::from_secs(5);
        let text = loop {
            match std::fs::read_to_string(&path) {
                Ok(text) => break text,
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(100))
                }
                Err(e) => return Err(format!("{}: {e}", path.display())),
            }
        };
        let v: Value =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let address = v["address"]
            .as_str()
            .and_then(|a| a.parse().ok())
            .ok_or_else(|| format!("{}: no address", path.display()))?;
        let token = v["token"].as_str().unwrap_or_default().to_owned();
        Ok(Some(Self { address, token }))
    }

    pub fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, String> {
        self.call_within(method, path, body, Duration::from_secs(60))
    }

    /// A call that may take long (a backup of a big world).
    pub fn call_within(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
        timeout: Duration,
    ) -> Result<Value, String> {
        let (status, text) = http::request(self.address, &self.token, method, path, body, timeout)
            .map_err(|e| format!("the server didn't answer: {e}"))?;
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if status == 200 {
            Ok(value)
        } else {
            Err(value["error"].as_str().unwrap_or(&text).to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn backup_server(dir: &Path) -> Arc<Shared> {
        std::fs::create_dir_all(dir.join("world")).unwrap();
        std::fs::write(dir.join("world/level.dat"), b"active-world-42").unwrap();
        std::fs::write(dir.join("server.toml"), crate::config::TEMPLATE).unwrap();
        let shared = Shared::with_install(dir, None);
        {
            let mut state = shared.state.lock().unwrap();
            state.phase = Phase::Running;
            state.world_name = Some("world".into());
        }
        shared
    }

    #[test]
    fn backups_follow_the_active_world_and_reject_transitions() {
        use crate::test_support::{Engine, TempDir};
        let dir = TempDir::new("active-world");
        let shared = backup_server(&dir);
        let peer = Engine::start(&dir, |line| {
            let (command, id) = line.split_once(' ').unwrap();
            assert!(matches!(command, "backup-begin" | "backup-end"));
            json!({"ok": true, "snapshot": id, "world": "world"})
        });
        for name in ["world", "absent-world", "different-world"] {
            if name == "different-world" {
                std::fs::create_dir(dir.join(name)).unwrap();
                std::fs::write(dir.join(name).join("level.dat"), b"wrong-world-93").unwrap();
            }
            let mut config = Config::default();
            config.world.name = name.into();
            std::fs::write(dir.join("server.toml"), crate::config::render(&config)).unwrap();
            let path = shared.back_up(&format!("{name}.zip")).unwrap();
            let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
            let mut marker = String::new();
            std::io::Read::read_to_string(
                &mut zip.by_name("world/level.dat").unwrap(),
                &mut marker,
            )
            .unwrap();
            assert_eq!(marker, "active-world-42");
            assert!(!zip.file_names().any(|p| p.starts_with("different-world/")));
        }
        assert_eq!(peer.commands.lock().unwrap().len(), 6);
        for phase in [Phase::Starting, Phase::Stopping, Phase::Restarting] {
            shared.set_phase(phase);
            let error = shared.back_up("transition.zip").unwrap_err();
            assert!(error.contains("isn't running"), "{error}");
        }
        assert!(!dir.join("backups/transition.zip").exists());
        assert_eq!(peer.commands.lock().unwrap().len(), 6);
    }

    #[test]
    fn restore_rejects_a_pending_world_name_change() {
        use crate::test_support::TempDir;
        let dir = TempDir::new("pending-world-restore");
        let shared = backup_server(&dir);
        let config = Config::default();
        backup::create(&dir, &config, "active.zip").unwrap();
        let mut pending = config;
        pending.world.name = "next-world".into();
        std::fs::write(dir.join("server.toml"), crate::config::render(&pending)).unwrap();

        let response = route(
            &shared,
            &http::Request {
                method: "POST".into(),
                path: "/api/v1/restore".into(),
                query: String::new(),
                headers: Vec::new(),
                body: json!({"name": "active.zip"}).to_string().into_bytes(),
            },
        );
        assert_eq!(response.status, 409, "{}", response.body);
        assert!(!shared.restart.load(Ordering::SeqCst));
    }

    #[test]
    fn accepted_restore_survives_pruning_with_a_positive_control() {
        use crate::test_support::TempDir;
        use std::time::{Duration, SystemTime};
        let dir = TempDir::new("restore-pruning");
        let shared = backup_server(&dir);
        let config = Config::default();
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs(1_792_000_000);
        let pinned = backup::auto_name(t);
        backup::create(&dir, &config, &pinned).unwrap();
        let eligible = backup::auto_name(t + Duration::from_secs(60));
        let newest = backup::auto_name(t + Duration::from_secs(120));
        std::fs::write(dir.join("backups").join(&eligible), b"eligible").unwrap();
        std::fs::write(dir.join("backups").join(&newest), b"newest").unwrap();

        let response = route(
            &shared,
            &http::Request {
                method: "POST".into(),
                path: "/api/v1/restore".into(),
                query: String::new(),
                headers: Vec::new(),
                body: json!({"name": pinned}).to_string().into_bytes(),
            },
        );
        assert_eq!(response.status, 200, "{}", response.body);

        shared.prune_backups(1);
        assert!(
            dir.join("backups").join(&pinned).exists(),
            "accepted restore was pruned"
        );
        assert!(dir.join("backups").join(&newest).exists());
        assert!(
            !dir.join("backups").join(&eligible).exists(),
            "positive control was retained"
        );
    }

    #[test]
    fn an_unacknowledged_snapshot_is_released_without_publishing() {
        use crate::test_support::{Engine, TempDir};
        for response in ["wrong-id", "wrong-world", "save-failed"] {
            let dir = TempDir::new(response);
            let shared = backup_server(&dir);
            let peer = Engine::start(&dir, move |line| {
                let (command, id) = line.split_once(' ').unwrap();
                if command == "backup-end" {
                    return json!({"ok": true, "snapshot": id});
                }
                match response {
                    "wrong-id" => json!({"ok": true, "snapshot": "stale", "world": "world"}),
                    "wrong-world" => json!({"ok": true, "snapshot": id, "world": "elsewhere"}),
                    _ => json!({"error": "disk write failed"}),
                }
            });
            let error = shared.back_up("rejected.zip").unwrap_err();
            let expected = match response {
                "wrong-id" => "did not acknowledge this backup snapshot",
                "wrong-world" => "acknowledged a different world",
                _ => "disk write failed",
            };
            assert!(error.contains(expected), "{error}");
            assert!(backup::list(&dir).is_empty());
            let commands = peer.commands.lock().unwrap();
            assert_eq!(commands.len(), 2);
            assert_eq!(
                commands[1],
                commands[0].replacen("backup-begin", "backup-end", 1)
            );
        }
    }

    #[test]
    fn failed_snapshot_and_failed_release_both_reach_the_caller() {
        let dir = crate::test_support::TempDir::new("snapshot-errors");
        let shared = backup_server(&dir);
        let peer = crate::test_support::Engine::start(
            &dir,
            |line| json!({"error": if line.starts_with("backup-begin ") { "save failure" } else { "release failure" }}),
        );
        let error = shared.back_up("failed.zip").unwrap_err();
        assert!(
            error.contains("save failure") && error.contains("release failure"),
            "{error}"
        );
        assert!(backup::list(&dir).is_empty());
        let commands = peer.commands.lock().unwrap();
        assert_eq!(commands.len(), 3);
        assert_eq!(commands[1], commands[2]);
    }

    #[test]
    fn log_keeps_the_last_lines_numbered() {
        let mut log = Log::default();
        for i in 0..LOG_LINES + 10 {
            log.push(i.to_string());
        }
        let (next, lines) = log.since(0);
        assert_eq!(next, (LOG_LINES + 10) as u64);
        assert_eq!(lines.len(), LOG_LINES);
        assert_eq!(lines[0], "10");
        let (next2, tail) = log.since(next - 2);
        assert_eq!(
            (next2, tail),
            (
                next,
                vec![(LOG_LINES + 8).to_string(), (LOG_LINES + 9).to_string()]
            )
        );
    }

    #[test]
    fn the_api_wants_its_token() {
        let dir = std::env::temp_dir().join(format!("horizon-server-api-{}", std::process::id()));
        std::fs::create_dir_all(dir.join(".horizon")).unwrap();
        let shared = Shared::new(&dir);
        let _endpoint = start(shared.clone()).unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(ENDPOINT)).unwrap()).unwrap();
        let address: SocketAddr = v["address"].as_str().unwrap().parse().unwrap();
        let token = v["token"].as_str().unwrap();
        let get = |token: &str| {
            http::request(
                address,
                token,
                "GET",
                "/api/v1/status",
                None,
                Duration::from_secs(5),
            )
            .unwrap()
        };
        assert_eq!(get("nope").0, 401);
        let (status, body) = get(token);
        assert_eq!(status, 200);
        let status: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(status["phase"], "starting");
        let (code, _) = http::request(
            address,
            token,
            "POST",
            "/api/v1/stop",
            None,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(code, 200);
        assert!(shared.stop.load(Ordering::SeqCst));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(ENDPOINT))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        drop(_endpoint);
        assert!(!dir.join(ENDPOINT).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
