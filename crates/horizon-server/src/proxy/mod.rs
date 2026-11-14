//! `[proxy]`: the public port belongs to the supervisor, and the game listens on 127.0.0.1 behind it.
//!
//! The proxy logs each player in itself (handshake, status, online-mode login with its encryption and
//! compression: auth.rs), then logs into the game as that player through Velocity modern forwarding
//! (backend.rs) and relays packets both ways (session.rs). It keeps what a later login to the game must be
//! told again (replay.rs). Not a general proxy: one game, the one this supervisor runs.

mod auth;
mod backend;
#[cfg(test)]
mod join_tests;
mod replay;
mod session;
mod wire;

use crate::config::Config;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use wire::{Connection, handshake, status};

/// The secret the proxy signs forwarded players with, beside `server.properties`: the file the game reads.
const SECRET_FILE: &str = "forwarding.secret";
/// Where the proxy keeps the secret it made, so it can tell that file from one an owner put there.
const OWN_SECRET: &str = ".horizon/proxy-forwarding.secret";
/// How long a client may take over its handshake and login.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

pub struct Inner {
    /// The game, on this machine.
    pub game: SocketAddr,
    pub secret: Vec<u8>,
    /// The threshold the proxy compresses its clients' connections with.
    pub compression: i32,
    keys: Option<auth::Keys>,
    /// Whether a game that answers is running: the server list asks it only then.
    up: AtomicBool,
    log: Mutex<Box<dyn Fn(&str, &str) + Send>>,
}

impl Inner {
    pub fn note(&self, level: &str, line: &str) {
        (self.log.lock().unwrap_or_else(|e| e.into_inner()))(level, line);
    }
}

/// The running proxy: its listener's thread stays until the supervisor ends.
#[derive(Clone)]
pub struct Proxy {
    inner: Arc<Inner>,
}

/// What the game is told so it listens where only the proxy goes: `server.properties` keys.
pub fn game_properties(game_port: u16, compression_off: bool) -> Vec<(String, String)> {
    let mut out = vec![
        ("server-ip".to_owned(), "127.0.0.1".to_owned()),
        ("server-port".to_owned(), game_port.to_string()),
        // The proxy has signed the player in: the game trusts what it forwards (online-mode=false).
        ("online-mode".to_owned(), "false".to_owned()),
    ];
    if compression_off {
        // Loopback: the proxy compresses for its clients, the game need not for the proxy.
        out.push(("network-compression-threshold".to_owned(), "-1".to_owned()));
    }
    out
}

/// Starts listening on the public address. `dir` is the server folder; `log` receives (level, line).
pub fn start(
    dir: &Path,
    config: &Config,
    log: impl Fn(&str, &str) + Send + 'static,
) -> Result<Proxy, String> {
    let bind: IpAddr = if config.server.bind.is_empty() {
        Ipv4Addr::UNSPECIFIED.into()
    } else {
        config
            .server
            .bind
            .parse()
            .map_err(|_| format!("server.bind {:?} is not an IP address", config.server.bind))?
    };
    let listener = TcpListener::bind((bind, config.server.port))
        .map_err(|e| format!("[proxy] can't listen on {bind}:{}: {e}", config.server.port))?;
    let secret = secret(dir)?;
    let keys = if config.server.online_mode {
        Some(auth::Keys::generate()?)
    } else {
        None
    };
    let inner = Arc::new(Inner {
        game: SocketAddr::from((Ipv4Addr::LOCALHOST, config.proxy.game_port)),
        secret,
        compression: compression_threshold(config),
        keys,
        up: AtomicBool::new(false),
        log: Mutex::new(Box::new(log)),
    });
    let accepting = inner.clone();
    thread::Builder::new()
        .name("Proxy listener".into())
        .spawn(move || accept(listener, &accepting))
        .map_err(|e| e.to_string())?;
    Ok(Proxy { inner })
}

impl Proxy {
    /// The game answers (`true`) or is down or still starting (`false`).
    pub fn set_game_up(&self, up: bool) {
        self.inner.up.store(up, Ordering::SeqCst);
    }
}

/// `network-compression-threshold` as the owner set it, vanilla's 256 otherwise.
fn compression_threshold(config: &Config) -> i32 {
    config
        .properties
        .get("network-compression-threshold")
        .and_then(|v| v.as_integer())
        .and_then(|v| i32::try_from(v).ok())
        .unwrap_or(256)
}

/// The shared secret: made once and kept, and put where the game reads it. A `forwarding.secret` the owner
/// wrote for a Velocity of their own is never overwritten: that and the proxy can't both be in front of the game.
fn secret(dir: &Path) -> Result<Vec<u8>, String> {
    let (own, file): (PathBuf, PathBuf) = (dir.join(OWN_SECRET), dir.join(SECRET_FILE));
    let kept = std::fs::read(&own)
        .ok()
        .map(|s| s.trim_ascii().to_vec())
        .filter(|s| !s.is_empty());
    let secret = match kept {
        Some(s) => s,
        None => {
            let mut random = [0u8; 24];
            getrandom::fill(&mut random)
                .map_err(|e| format!("no random numbers for the proxy's secret: {e}"))?;
            let hex: String = random.iter().map(|b| format!("{b:02x}")).collect();
            if let Some(parent) = own.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::write(&own, &hex).map_err(|e| format!("{}: {e}", own.display()))?;
            hex.into_bytes()
        }
    };
    match std::fs::read(&file) {
        Ok(existing) if existing.trim_ascii() != secret.as_slice() => {
            return Err(format!(
                "[proxy] is on but {SECRET_FILE} holds another secret (a Velocity's?): remove the file, or turn [proxy] off"
            ));
        }
        Ok(_) => {}
        Err(_) => std::fs::write(&file, &secret).map_err(|e| format!("{}: {e}", file.display()))?,
    }
    Ok(secret)
}

/// With `[proxy]` off again, the file the proxy put there goes, or the game would refuse everyone who joins
/// without a proxy.
pub fn remove_own_secret(dir: &Path) {
    let own = std::fs::read(dir.join(OWN_SECRET)).ok();
    let file = dir.join(SECRET_FILE);
    if let (Some(own), Ok(existing)) = (own, std::fs::read(&file))
        && own.trim_ascii() == existing.trim_ascii()
    {
        let _ = std::fs::remove_file(file);
    }
}

fn accept(listener: TcpListener, inner: &Arc<Inner>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let inner = inner.clone();
        let _ = thread::Builder::new()
            .name("Proxy client".into())
            .spawn(move || {
                if let Err(e) = serve(stream, &inner) {
                    inner.note("DEBUG", &format!("A connection to the proxy ended: {e}"));
                }
            });
    }
}

fn serve(stream: TcpStream, inner: &Arc<Inner>) -> Result<(), String> {
    let peer = stream.peer_addr().map_err(|e| e.to_string())?;
    let mut c = Connection::from_stream(stream).map_err(|e| e.to_string())?;
    c.reader
        .set_read_timeout(Some(HANDSHAKE_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let e = |e: wire::Error| e.to_string();
    let (id, payload) = c.recv().map_err(e)?;
    let hello =
        handshake::decode_intention(id, &payload, handshake::PROXY_HOST_LIMIT).map_err(e)?;
    match hello.intent {
        handshake::STATUS => serve_status(&mut c, inner, hello.protocol),
        handshake::LOGIN | handshake::TRANSFER => {
            if hello.protocol != wire::PROTOCOL_VERSION {
                let reason = wire::translated_json(
                    "multiplayer.disconnect.incompatible",
                    "Incompatible client! Please use the matching version",
                );
                let _ = c.send(
                    &wire::login::Clientbound::Disconnect(wire::login::Disconnect {
                        reason_json: reason,
                    })
                    .encode(),
                );
                return Ok(());
            }
            serve_login(c, inner, peer.ip())
        }
        other => Err(format!("unknown handshake intent {other}")),
    }
}

fn serve_login(mut c: Connection, inner: &Arc<Inner>, peer: IpAddr) -> Result<(), String> {
    let e = |e: wire::Error| e.to_string();
    let (id, payload) = c.recv().map_err(e)?;
    let name = match wire::login::Serverbound::decode(id, &payload).map_err(e)? {
        wire::login::Serverbound::Hello(h) => h.name,
        _ => return Err("expected a login hello".into()),
    };
    if !auth::valid_name(&name) {
        return Err(format!("{name:?} is not a player name"));
    }
    match auth::profile(&mut c, inner.keys.as_ref(), &name)? {
        Ok(who) => {
            session::run(inner, c, who, peer);
            Ok(())
        }
        Err(reason_json) => {
            let _ = c.send(
                &wire::login::Clientbound::Disconnect(wire::login::Disconnect { reason_json })
                    .encode(),
            );
            Ok(())
        }
    }
}

/// The server list: the game's own answer, or one that says it is restarting.
fn serve_status(c: &mut Connection, inner: &Arc<Inner>, protocol: i32) -> Result<(), String> {
    let e = |e: wire::Error| e.to_string();
    loop {
        let (id, payload) = c.recv().map_err(e)?;
        match status::Serverbound::decode(id, &payload).map_err(e)? {
            status::Serverbound::StatusRequest(_) => {
                let json = inner
                    .up
                    .load(Ordering::SeqCst)
                    .then(|| backend::status_json(inner.game).ok())
                    .flatten()
                    .unwrap_or_else(|| restarting_status(protocol));
                c.send(
                    &status::Clientbound::StatusResponse(status::StatusResponse { json }).encode(),
                )
                .map_err(e)?;
            }
            status::Serverbound::PingRequest(ping) => {
                c.send(&status::Clientbound::PongResponse(ping).encode())
                    .map_err(e)?;
                return Ok(());
            }
            status::Serverbound::Unknown(..) => return Err("unknown status packet".into()),
        }
    }
}

/// What the server list shows while the game is down.
fn restarting_status(protocol: i32) -> String {
    serde_json::json!({
        "version": { "name": wire::version_name(), "protocol": protocol },
        "players": { "max": 0, "online": 0 },
        "description": { "translate": "horizon.proxy.restarting", "fallback": "Restarting, back in a moment" },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_restarting_status_is_translated_and_valid_json() {
        let v: serde_json::Value = serde_json::from_str(&restarting_status(777)).unwrap();
        assert_eq!(v["description"]["translate"], "horizon.proxy.restarting");
        assert_eq!(v["version"]["protocol"], 777);
    }

    #[test]
    fn the_game_listens_only_for_the_proxy_and_trusts_what_it_forwards() {
        let p = game_properties(25599, true);
        let get = |k: &str| p.iter().find(|(key, _)| key == k).map(|(_, v)| v.as_str());
        assert_eq!(get("server-ip"), Some("127.0.0.1"));
        assert_eq!(get("server-port"), Some("25599"));
        assert_eq!(get("online-mode"), Some("false"));
    }

    #[test]
    fn a_secret_that_is_not_ours_stops_the_start_and_ours_is_kept() {
        let dir = std::env::temp_dir().join(format!("horizon-proxy-secret-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(SECRET_FILE), "velocitys-own").unwrap();
        assert!(secret(&dir).unwrap_err().contains("another secret"));
        std::fs::remove_file(dir.join(SECRET_FILE)).unwrap();
        let first = secret(&dir).unwrap();
        assert_eq!(secret(&dir).unwrap(), first, "kept across starts");
        assert_eq!(std::fs::read(dir.join(SECRET_FILE)).unwrap(), first);
        remove_own_secret(&dir);
        assert!(!dir.join(SECRET_FILE).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
