//! The auto-reconnect extra (`docs/extras.md`): whether a lost multiplayer
//! connection may be retried, and when. Only lost connections and the
//! server's own shutdown or restart are retried, never kicks, bans or
//! refusals; attempts back off and stop after a few.
//!
//! It runs on the engine's connection hooks ([`HOOKS`], `horizon_hooks`): the engine
//! reports a connection's start, join, leave and end, and shows and runs the attempt
//! this plans. Nothing else in the engine reads it, so it leaves for a Horizon crate
//! as a file move once its setting can leave too (docs/layers.md, plan M5).
use horizon_hooks::{Ended, Hooks, Server, seat};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A planned attempt.
pub use horizon_hooks::Reconnect as Retry;

/// Seconds before each attempt; there are no more after the last.
pub const DELAYS: [u64; 5] = [5, 10, 20, 40, 60];
/// Staying connected this long counts the next drop as a fresh one.
pub const STABLE: Duration = Duration::from_secs(60);

/// Whether a disconnect may be retried. `from_server`: the server (or a
/// permanent refusal) gave `reason`, rather than the connection failing.
///
/// `key`: the reason's translation key, when it was a translatable
/// component or a known Horizon error. A key the classification knows
/// decides; the English keyword scan of the text is only the fallback for
/// plain reasons (OS errors, servers' literal kicks), whose language is
/// the server's own.
pub fn allowed(reason: &str, key: Option<&str>, from_server: bool) -> bool {
    if let Some(decision) = key.and_then(classified_key) {
        return decision;
    }
    let r = reason.to_lowercase();
    // Kicks, bans, whitelists, version refusals and the account's session
    // refusals (`disconnect.loginFailedInfo.*` but the unreachable session
    // servers, the server's failed username check, joining an online-mode
    // server without an account): never.
    const NEVER: [&str; 13] = [
        "ban",
        "kick",
        "white-list",
        "whitelist",
        "incompatible",
        "outdated",
        "another location",
        "online mode",
        "invalid server address",
        "invalid session",
        "multiplayer is disabled",
        "failed to verify username",
        "microsoft account",
    ];
    if NEVER.iter().any(|w| r.contains(w)) {
        return false;
    }
    if !from_server {
        return true;
    }
    // The server going away for a while: vanilla's shutdown and time-out
    // reasons, a full server, a slow login, the server's own session
    // servers being down, and restart messages.
    const RETRY: [&str; 6] = [
        "server closed",
        "timed out",
        "the server is full",
        "took too long to log in",
        "authentication servers",
        "restart",
    ];
    RETRY.iter().any(|w| r.contains(w))
}

/// The key lists' key-based counterpart: `Some` when the key alone
/// decides. Prefix match, so a key's arguments' variants
/// (`multiplayer.disconnect.banned.expiration`) share the base's verdict.
fn classified_key(key: &str) -> Option<bool> {
    // The keyword lists above, as the keys those texts carry: bans, kicks,
    // whitelists, version refusals, the account's session refusals and
    // Horizon's own refusals: never. Shutdowns, timeouts, a full server, a
    // slow login and the session servers being down: retry.
    const NEVER: [&str; 14] = [
        "multiplayer.disconnect.banned",
        "multiplayer.disconnect.ip_banned",
        "multiplayer.disconnect.kicked",
        "multiplayer.disconnect.not_whitelisted",
        "multiplayer.disconnect.incompatible",
        "multiplayer.disconnect.outdated_client",
        "multiplayer.disconnect.outdated_server",
        "multiplayer.disconnect.duplicate_login",
        "multiplayer.disconnect.unverified_username",
        "disconnect.loginFailedInfo.invalidSession",
        "disconnect.loginFailedInfo.insufficientPrivileges",
        "disconnect.loginFailedInfo.userBanned",
        "horizon.multiplayer.not_signed_in",
        "horizon.multiplayer.invalid_address",
    ];
    const RETRY: [&str; 6] = [
        "multiplayer.disconnect.server_shutdown",
        "multiplayer.disconnect.server_full",
        "multiplayer.disconnect.slow_login",
        "multiplayer.disconnect.authservers_down",
        "disconnect.loginFailedInfo.serversUnavailable",
        "disconnect.timeout",
    ];
    NEVER
        .iter()
        .any(|k| key.starts_with(k))
        .then_some(false)
        .or_else(|| RETRY.iter().any(|k| key.starts_with(k)).then_some(true))
}

/// Attempts at one address.
#[derive(Debug, Default)]
pub struct Tracker {
    address: Option<String>,
    /// The player name the connection used.
    name: String,
    lan: bool,
    /// Attempts already made since the last stable connection.
    attempts: usize,
    joined: Option<Instant>,
}
impl Tracker {
    /// A connection to `address` as `name` starts: a new address starts
    /// counting anew.
    pub fn connecting(&mut self, address: &str, name: &str, lan: bool) {
        if self.address.as_deref() != Some(address) {
            self.address = Some(address.to_owned());
            self.attempts = 0;
        }
        self.name = name.to_owned();
        self.lan = lan;
        self.joined = None;
    }
    /// The server let the player in.
    pub fn joined(&mut self, now: Instant) {
        self.joined = Some(now);
    }
    /// The player left on purpose, or cancelled: nothing to retry.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    /// The connection ended: the next attempt, or None (not allowed, or
    /// out of attempts).
    pub fn ended(
        &mut self,
        reason: &str,
        key: Option<&str>,
        from_server: bool,
        now: Instant,
    ) -> Option<Retry> {
        if self
            .joined
            .take()
            .is_some_and(|t| now.saturating_duration_since(t) >= STABLE)
        {
            self.attempts = 0;
        }
        let address = self.address.clone()?;
        if !allowed(reason, key, from_server) || self.attempts >= DELAYS.len() {
            self.reset();
            return None;
        }
        let delay = Duration::from_secs(DELAYS[self.attempts]);
        self.attempts += 1;
        Some(Retry {
            address,
            name: self.name.clone(),
            lan: self.lan,
            at: now + delay,
            attempt: self.attempts,
            attempts: DELAYS.len(),
        })
    }
}

/// Auto-reconnect on the engine's connection hooks: registered at startup with the other
/// extras (`entry.rs`); it plans nothing while its setting is off. Splitscreen players share
/// the hooks, so each seat (`horizon_hooks::seat`) has its own tracker and setting, as each
/// player's game had before.
pub static HOOKS: AutoReconnect = AutoReconnect {
    seats: Mutex::new(Vec::new()),
};

pub struct AutoReconnect {
    seats: Mutex<Vec<Seat>>,
}

#[derive(Default)]
struct Seat {
    tracker: Tracker,
    enabled: bool,
}

impl AutoReconnect {
    /// The `autoReconnect` setting of the player whose turn it is.
    pub fn set_enabled(&self, on: bool) {
        self.with(|s| s.enabled = on);
    }
    fn with<T>(&self, f: impl FnOnce(&mut Seat) -> T) -> T {
        let mut seats = self.seats.lock().unwrap_or_else(|e| e.into_inner());
        let i = seat();
        if seats.len() <= i {
            seats.resize_with(i + 1, Seat::default);
        }
        f(&mut seats[i])
    }
}

impl Hooks for AutoReconnect {
    fn connecting(&self, server: &Server<'_>) {
        self.with(|s| {
            s.tracker
                .connecting(server.address, server.name, server.lan)
        });
    }
    fn joined(&self) {
        self.with(|s| s.tracker.joined(Instant::now()));
    }
    fn left(&self) {
        self.with(|s| s.tracker.reset());
    }
    fn ended(&self, end: &Ended<'_>) -> Option<Retry> {
        self.with(|s| {
            if !s.enabled {
                s.tracker.reset();
                return None;
            }
            s.tracker
                .ended(end.reason, end.key, end.from_server, Instant::now())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_keeps_the_selected_server_kind() {
        let mut tracker = Tracker::default();
        let now = Instant::now();
        tracker.connecting("127.0.0.1:25565", "Tester", true);
        assert!(tracker.ended("Server closed", None, true, now).unwrap().lan);
        tracker.connecting("127.0.0.1:25565", "Tester", false);
        assert!(!tracker.ended("Server closed", None, true, now).unwrap().lan);
    }

    #[test]
    fn only_lost_connections_and_shutdowns_are_retried() {
        assert!(allowed(
            "Connection reset by peer (os error 104)",
            None,
            false
        ));
        assert!(allowed("Connection refused (os error 111)", None, false));
        assert!(allowed("Server closed", None, true));
        assert!(allowed("Timed out", None, true));
        assert!(allowed("The server is full!", None, true));
        assert!(allowed("Server is restarting, back soon", None, true));
        assert!(!allowed("Kicked by an operator", None, true));
        assert!(!allowed(
            "You are banned from this server.\nReason: griefing",
            None,
            true
        ));
        assert!(!allowed(
            "You are not white-listed on this server!",
            None,
            true
        ));
        assert!(!allowed("You logged in from another location", None, true));
        assert!(!allowed("Incompatible client! Please use 26.3", None, true));
        assert!(!allowed("You have been idle for too long!", None, true));
        assert!(
            !allowed("Go away", None, true),
            "unknown server reasons are not retried"
        );
        assert!(!allowed(
            "This server is in online mode. Horizon only joins offline-mode servers.",
            Some("horizon.multiplayer.not_signed_in"),
            true
        ));
        assert!(!allowed("Kicked for spamming", None, false));
    }

    #[test]
    fn online_mode_login_failures() {
        // The client's join with the session service (net.rs: Err, with
        // vanilla's disconnect.loginFailedInfo texts and the inner key).
        let failed = |why: &str| format!("Failed to log in: {why}");
        assert!(!allowed(
            &failed("Invalid session (Try restarting your game and the launcher)"),
            Some("disconnect.loginFailedInfo.invalidSession"),
            false
        ));
        assert!(!allowed(
            &failed("You are banned from playing online"),
            Some("disconnect.loginFailedInfo.userBanned"),
            false
        ));
        assert!(!allowed(
            &failed("Multiplayer is disabled. Please check your Microsoft account settings."),
            Some("disconnect.loginFailedInfo.insufficientPrivileges"),
            false
        ));
        assert!(!allowed(
            crate::net::NOT_SIGNED_IN,
            Some("horizon.multiplayer.not_signed_in"),
            false
        ));
        // Unreachable session servers are worth another try.
        assert!(allowed(
            &failed("The authentication servers are currently not reachable. Please try again."),
            Some("disconnect.loginFailedInfo.serversUnavailable"),
            false
        ));
        assert!(allowed(&failed("Connection timed out"), None, false));
        // The server's side: its username check failed, or its session
        // servers are down.
        assert!(!allowed("Failed to verify username!", None, true));
        assert!(allowed(
            "Authentication servers are down. Please try again later. Sorry!",
            Some("multiplayer.disconnect.authservers_down"),
            true
        ));
    }

    #[test]
    fn keys_decide_whatever_language_resolved_the_text() {
        // A zh_cn client resolves the server's and the session service's
        // texts: the keyword scan of the text would see none of the
        // English words, the keys do. A tiny fake language file stands in
        // for vanilla's; Horizon's bundled strings come with the map.
        use horizon_client_core::lang::{Language, ResourceSource};
        use std::collections::HashMap;
        struct Mem(HashMap<String, Vec<u8>>);
        impl ResourceSource for Mem {
            fn read(&self, path: &str) -> Option<Vec<u8>> {
                self.0.get(path).cloned()
            }
        }
        let mem = Mem(HashMap::from([(
            "assets/minecraft/lang/zh_cn.json".into(),
            r#"{
                "multiplayer.disconnect.server_shutdown": "服务器已关闭",
                "multiplayer.disconnect.server_full": "服务器已满！",
                "multiplayer.disconnect.slow_login": "登录超时",
                "multiplayer.disconnect.kicked": "被管理员踢出游戏",
                "multiplayer.disconnect.banned": "你已被此服务器封禁",
                "multiplayer.disconnect.not_whitelisted": "你不在此服务器的白名单中！",
                "disconnect.loginFailedInfo.invalidSession": "无效会话（请尝试重启游戏及启动器）"
            }"#
            .as_bytes()
            .to_vec(),
        )]));
        let lang = Language::load(&[&mem], "zh_cn");
        let zh = |key: &str| lang.get(key).unwrap_or(key).to_owned();
        assert_eq!(zh("multiplayer.disconnect.server_shutdown"), "服务器已关闭");
        assert_eq!(
            zh("horizon.multiplayer.not_signed_in"),
            "该服务器为在线模式服务器。请先在 Horizon 启动器中登录 Microsoft 账户再加入。"
        );
        // Shutdowns, a full server and a slow login are retried, kicks and
        // bans are not — whatever the resolved text says.
        assert!(allowed(
            &zh("multiplayer.disconnect.server_shutdown"),
            Some("multiplayer.disconnect.server_shutdown"),
            true
        ));
        assert!(allowed(
            &zh("multiplayer.disconnect.server_full"),
            Some("multiplayer.disconnect.server_full"),
            true
        ));
        assert!(allowed(
            &zh("multiplayer.disconnect.slow_login"),
            Some("multiplayer.disconnect.slow_login"),
            true
        ));
        assert!(!allowed(
            &zh("multiplayer.disconnect.kicked"),
            Some("multiplayer.disconnect.kicked"),
            true
        ));
        assert!(!allowed(
            &zh("multiplayer.disconnect.banned"),
            Some("multiplayer.disconnect.banned"),
            true
        ));
        assert!(!allowed(
            &zh("multiplayer.disconnect.not_whitelisted"),
            Some("multiplayer.disconnect.not_whitelisted"),
            true
        ));
        assert!(!allowed(
            &zh("disconnect.loginFailedInfo.invalidSession"),
            Some("disconnect.loginFailedInfo.invalidSession"),
            false
        ));
        assert!(!allowed(
            &zh("horizon.multiplayer.not_signed_in"),
            Some("horizon.multiplayer.not_signed_in"),
            true
        ));
        // Unknown and missing keys fall back to the text scan.
        assert!(allowed(
            "Connection reset by peer (os error 104)",
            Some("made.up.key"),
            false
        ));
        assert!(allowed("Connection refused (os error 111)", None, false));
        assert!(!allowed("Go away", Some("made.up.key"), true));
    }

    #[test]
    fn attempts_back_off_and_stop() {
        let mut t = Tracker::default();
        let now = Instant::now();
        t.connecting("localhost:25565", "Tester", false);
        let mut delays = Vec::new();
        while let Some(r) = t.ended("Connection refused", None, false, now) {
            assert_eq!(
                (r.address.as_str(), r.name.as_str()),
                ("localhost:25565", "Tester")
            );
            assert_eq!(r.attempt, delays.len() + 1);
            delays.push(r.at.duration_since(now).as_secs());
            t.connecting(&r.address, &r.name, false);
        }
        assert_eq!(delays, DELAYS);
        // Out of attempts, the tracker forgets the address.
        assert_eq!(t.ended("Connection refused", None, false, now), None);
    }

    #[test]
    fn a_short_stay_does_not_reset_the_count() {
        let mut t = Tracker::default();
        let now = Instant::now();
        t.connecting("a", "P", false);
        for _ in 0..DELAYS.len() {
            t.connecting("a", "P", false);
            t.joined(now);
            assert!(
                t.ended("Server closed", None, true, now + Duration::from_secs(5))
                    .is_some()
            );
        }
        t.connecting("a", "P", false);
        t.joined(now);
        assert_eq!(
            t.ended("Server closed", None, true, now + Duration::from_secs(5)),
            None
        );
        // A minute connected starts over.
        t.connecting("a", "P", false);
        for _ in 0..3 {
            t.ended("Timed out", None, true, now);
            t.connecting("a", "P", false);
        }
        t.joined(now);
        let r = t.ended("Timed out", None, true, now + STABLE).unwrap();
        assert_eq!(r.attempt, 1);
    }

    #[test]
    fn kicks_and_leaving_reset_and_new_addresses_start_over() {
        let mut t = Tracker::default();
        let now = Instant::now();
        t.connecting("a", "P", false);
        assert!(t.ended("Server closed", None, true, now).is_some());
        assert_eq!(t.ended("Kicked by an operator", None, true, now), None);
        assert_eq!(
            t.ended("Server closed", None, true, now),
            None,
            "forgotten after a kick"
        );
        t.connecting("a", "P", false);
        t.ended("Server closed", None, true, now);
        t.connecting("b", "P", false);
        assert_eq!(
            t.ended("Server closed", None, true, now).unwrap().attempt,
            1
        );
        let mut t = Tracker::default();
        assert_eq!(
            t.ended("Server closed", None, true, now),
            None,
            "never connected"
        );
    }

    #[test]
    fn each_splitscreen_seat_keeps_its_own_attempts_and_setting() {
        use horizon_hooks::engine::set_seat;
        let hooks = AutoReconnect {
            seats: Mutex::new(Vec::new()),
        };
        let lost = Ended::new("Timed out", Some("disconnect.timeout"), false);
        for (seat, on) in [(0, false), (1, true)] {
            set_seat(seat);
            hooks.set_enabled(on);
            hooks.connecting(&Server::new("example.org", "Player", false));
        }
        // P2 has it on: an attempt in 5 s. P1 has it off: none, and P2's state stays.
        set_seat(1);
        assert_eq!(hooks.ended(&lost).map(|r| r.attempt), Some(1));
        set_seat(0);
        assert_eq!(hooks.ended(&lost), None);
        set_seat(1);
        hooks.connecting(&Server::new("example.org", "Player", false));
        assert_eq!(hooks.ended(&lost).map(|r| r.attempt), Some(2));
        // P2 leaving on purpose starts it over.
        hooks.left();
        hooks.connecting(&Server::new("example.org", "Player", false));
        assert_eq!(hooks.ended(&lost).map(|r| r.attempt), Some(1));
        set_seat(0);
    }

    #[test]
    fn the_countdown_rounds_up() {
        let now = Instant::now();
        let r = Retry {
            address: "a".into(),
            name: "P".into(),
            lan: false,
            at: now + Duration::from_millis(4200),
            attempt: 2,
            attempts: DELAYS.len(),
        };
        assert_eq!(r.seconds_left(now), 5);
        assert_eq!(r.text(now), "Reconnecting in 5 s (attempt 2 of 5)");
        assert!(!r.due(now));
        assert!(r.due(now + Duration::from_secs(5)));
        assert_eq!(r.seconds_left(now + Duration::from_secs(9)), 0);
    }
}
