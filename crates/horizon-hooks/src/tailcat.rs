//! Tailcat multiplayer: WireGuard/DERP tunnels without a Tailscale account.
//!
//! Horizon runs the `tailcat-for-minecraft` Go helper, which speaks JSON events
//! on stdout. `host --target <loopback>` answers `ready` with an `mcl1_...`
//! invitation; `join --invite <invitation>` answers `ready` with a loopback
//! listener (`127.0.0.1:<port>`) that forwards to the host's server. Both
//! helpers proxy only the Minecraft port ([`PORT`]) and bind join listeners to
//! loopback, so the rest of Horizon sees a plain join.
//!
//! The helper is found from `HORIZON_TAILCAT_HELPER`, then a checked-out
//! `tailcat-for-minecraft` beside the game (development), then the cached copy,
//! then downloaded from the pinned upstream commit and checked against its
//! `checksums.json`.
//!
//! The engine reaches this through `horizon_hooks::tailcat`; the Horizon layer
//! reaches it through `horizon_platform::hooks::tailcat`. The game passes its
//! own cache directory ([`crate::logging`]'s game home) and the launcher and the
//! server pass theirs, so one implementation serves all of them. The address the
//! helper is downloaded from is the Horizon layer's
//! ([`horizon_addresses`](crate)), handed in with [`set_assets`].
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Every invitation starts with this (`tailcat-for-minecraft`'s format).
pub const INVITE_PREFIX: &str = "mcl1_";
/// The only port the helper proxies: the server's port on the tunnel.
pub const PORT: u16 = 25565;
/// How long a helper may take to pick a DERP region and start listening.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(25);
/// The `tailcat-for-minecraft` commit the helper binary is downloaded from.
const MCLINK_COMMIT: &str = "c561a896f3e48c98bde36b0a810b29f560b074a5";

static ASSETS: OnceLock<&'static str> = OnceLock::new();

/// Names the helper's download base (`crates/horizon-addresses`). The first call wins: a later
/// one is a second registration, not a change.
pub fn set_assets(url: &'static str) {
    let _ = ASSETS.set(url);
}

/// The base URL, once a Horizon crate set one. Without one (a plain build, the server build) the
/// helper can still be found on disk or in `HORIZON_TAILCAT_HELPER`, but not downloaded.
pub fn assets() -> Option<&'static str> {
    ASSETS.get().copied()
}

/// A running helper. Dropping it stops the helper: closing stdin ends its
/// context, and a helper still running two seconds later is killed.
pub struct Running {
    child: Child,
}

impl Running {
    /// Whether the helper process is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        // The helper treats stdin EOF as its stop signal.
        drop(self.child.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return;
                }
            }
        }
    }
}

/// Share a server on `server_port` (on loopback) behind a fresh invitation. The
/// helper is cached in `cache_dir` (the game's `tailcat` folder, the launcher's,
/// or the server's).
pub fn host(server_port: u16, cache_dir: &Path) -> Result<(Running, String), String> {
    let target = format!("127.0.0.1:{server_port}");
    let (running, events) = start(&["host", "--target", target.as_str()], cache_dir)?;
    match wait_ready(&events, "host")? {
        Ready::Invite(invite) => Ok((running, invite)),
        Ready::Address(_) => Err("the Tailcat helper answered in the wrong mode".to_owned()),
    }
}

/// Reach the world behind `invite`, returning the loopback address to join.
pub fn join(invite: &str, cache_dir: &Path) -> Result<(Running, String), String> {
    let invite = invite.trim();
    if !invite.starts_with(INVITE_PREFIX) {
        return Err("a Tailcat invitation starts with mcl1_".to_owned());
    }
    let (running, events) = start(&["join", "--invite", invite], cache_dir)?;
    match wait_ready(&events, "join")? {
        Ready::Address(address) if address.starts_with("127.0.0.1:") => Ok((running, address)),
        Ready::Address(address) => {
            Err(format!("the helper gave a non-loopback address: {address}"))
        }
        Ready::Invite(_) => Err("the Tailcat helper answered in the wrong mode".to_owned()),
    }
}

/// Spawn the helper and start draining its output: stdout's events to `events`,
/// stderr to the log.
fn start(args: &[&str], cache_dir: &Path) -> Result<(Running, mpsc::Receiver<String>), String> {
    let binary = helper_binary(cache_dir)?;
    let mut command = Command::new(&binary);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: the helper has no console of its own.
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("could not run the Tailcat helper {}: {e}", binary.display()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or("the Tailcat helper has no stdout")?;
    if let Some(stderr) = child.stderr.take() {
        drain_stderr(stderr);
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            // Keep draining after the receiver goes away so the pipe never fills.
            let _ = tx.send(line);
        }
    });
    Ok((Running { child }, rx))
}

/// Read events until `ready` for `mode`, or an `error`, or the helper exits.
fn wait_ready(events: &mpsc::Receiver<String>, mode: &str) -> Result<Ready, String> {
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(format!(
                "the Tailcat helper did not become ready within {} seconds",
                STARTUP_TIMEOUT.as_secs()
            ));
        }
        match events.recv_timeout(remaining) {
            Ok(line) => match Event::parse(&line) {
                Ok(event) if event.kind == "ready" => {
                    if event.mode.as_deref() != Some(mode) {
                        return Err("the Tailcat helper answered in the wrong mode".to_owned());
                    }
                    return if mode == "host" {
                        event
                            .invite
                            .map(Ready::Invite)
                            .ok_or_else(|| "the Tailcat helper sent no invitation".to_owned())
                    } else {
                        event
                            .address
                            .map(Ready::Address)
                            .ok_or_else(|| "the Tailcat helper sent no address".to_owned())
                    };
                }
                Ok(event) if event.kind == "error" => {
                    return Err(match (event.code, event.message) {
                        (Some(code), Some(message)) => format!("{code}: {message}"),
                        (_, Some(message)) => message,
                        (Some(code), None) => code,
                        (None, None) => "the Tailcat helper failed".to_owned(),
                    });
                }
                // "stopped" and anything else: keep waiting.
                Ok(_) => {}
                Err(error) => log::warn!("tailcat helper: {error}"),
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err("the Tailcat helper exited before it was ready".to_owned());
            }
        }
    }
}

enum Ready {
    Invite(String),
    Address(String),
}

/// A line of the helper's stdout.
#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    invite: Option<String>,
    #[serde(default)]
    address: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

impl Event {
    fn parse(line: &str) -> Result<Self, String> {
        serde_json::from_str(line.trim()).map_err(|e| e.to_string())
    }
}

fn drain_stderr(stderr: ChildStderr) {
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines() {
            match line {
                Ok(line) => log::info!("tailcat helper: {line}"),
                Err(_) => break,
            }
        }
    });
}

/// The helper executable: `HORIZON_TAILCAT_HELPER`, a checked-out
/// `tailcat-for-minecraft`, the cached copy, then a download.
pub fn helper_binary(cache_dir: &Path) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("HORIZON_TAILCAT_HELPER").filter(|p| !p.is_empty()) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "HORIZON_TAILCAT_HELPER is not a file: {}",
            path.display()
        ));
    }
    if let Some(path) = clone_helper() {
        return Ok(path);
    }
    let cached = cached_helper(cache_dir);
    if cached.is_file() {
        return Ok(cached);
    }
    install(cache_dir)
}

/// The platform folder in `tailcat-for-minecraft`'s native assets.
fn platform() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("linux-amd64"),
        ("linux", "aarch64") => Ok("linux-arm64"),
        ("macos", "x86_64") => Ok("darwin-amd64"),
        ("macos", "aarch64") => Ok("darwin-arm64"),
        ("windows", "x86_64") => Ok("windows-amd64"),
        ("windows", "aarch64") => Ok("windows-arm64"),
        (os, arch) => Err(format!("Tailcat has no helper for {os}/{arch}")),
    }
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "mclink-helper.exe"
    } else {
        "mclink-helper"
    }
}

/// A `tailcat-for-minecraft` checked out beside the game (development).
fn clone_helper() -> Option<PathBuf> {
    let platform = platform().ok()?;
    let path = std::env::current_dir()
        .ok()?
        .join("tailcat-for-minecraft/mod/src/main/resources/assets/mclink/native")
        .join(platform)
        .join(exe_name());
    path.is_file().then_some(path)
}

fn cached_helper(cache_dir: &Path) -> PathBuf {
    cache_dir.join(exe_name())
}

/// Download the helper for this platform and check it against the commit's
/// `checksums.json`.
fn install(cache_dir: &Path) -> Result<PathBuf, String> {
    let Some(assets) = assets() else {
        return Err("this build has no Tailcat helper download address".into());
    };
    let platform = platform()?;
    let base = format!("{assets}/{MCLINK_COMMIT}/mod/src/main/resources/assets/mclink/native");
    let sums = get(&format!("{base}/checksums.json"))?;
    let sums = String::from_utf8(sums).map_err(|e| e.to_string())?;
    let expected = checksum_for(&sums, &format!("{platform}/{}", exe_name()))?;
    let bytes = get(&format!("{base}/{platform}/{}", exe_name()))?;
    let actual = hex(Sha256::digest(&bytes).as_slice());
    if actual != expected.to_ascii_lowercase() {
        return Err(format!(
            "the downloaded Tailcat helper did not match its checksum (expected {expected}, got {actual})"
        ));
    }
    let path = cached_helper(cache_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    log::info!("Downloaded the Tailcat helper to {}", path.display());
    Ok(path)
}

/// One `"<platform>/<exe>": "<sha256>"` entry of `checksums.json`.
fn checksum_for(json: &str, key: &str) -> Result<String, String> {
    let sums: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    sums.get(key)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .ok_or_else(|| format!("checksums.json has no entry for {key}"))
}

fn get(url: &str) -> Result<Vec<u8>, String> {
    let response = ureq::get(url)
        .set("User-Agent", "horizon-tailcat")
        .call()
        .map_err(|e| format!("download {url}: {e}"))?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_helper_events() {
        let host = Event::parse(r#"{"type":"ready","mode":"host","invite":"mcl1_abc"}"#).unwrap();
        assert_eq!(host.kind, "ready");
        assert_eq!(host.mode.as_deref(), Some("host"));
        assert_eq!(host.invite.as_deref(), Some("mcl1_abc"));

        let join =
            Event::parse(r#"{"type":"ready","mode":"join","address":"127.0.0.1:1234"}"#).unwrap();
        assert_eq!(join.address.as_deref(), Some("127.0.0.1:1234"));

        let error =
            Event::parse(r#"{"type":"error","code":"invalid_invite","message":"bad"}"#).unwrap();
        assert_eq!(error.code.as_deref(), Some("invalid_invite"));
    }

    #[test]
    fn finds_a_checksum_by_platform_path() {
        let sums = r#"{"darwin-arm64/mclink-helper":"ABC123"}"#;
        assert_eq!(
            checksum_for(sums, "darwin-arm64/mclink-helper").unwrap(),
            "ABC123"
        );
        assert!(checksum_for(sums, "linux-amd64/mclink-helper").is_err());
    }

    #[test]
    fn refuses_a_non_invitation_before_running_anything() {
        let error = join("https://example.com", Path::new(".")).err().unwrap();
        assert!(error.contains(INVITE_PREFIX), "{error}");
    }
}
