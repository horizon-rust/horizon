//! Play with friends, host side: "Start Sharing" on the Play with Friends
//! screen (`screen`, the pause menu's Friends button) opens the world to LAN if
//! it isn't (with Open to LAN's defaults) and shares that port behind an
//! invite code (horizon-friends). The code holds this computer's addresses
//! and the key, so friends join with no server of ours; a relay the player
//! configured adds a short code and a fallback. The integrated server is the
//! LAN one, unchanged: tunnelled guests are LAN guests that connect from the
//! loopback address. The joining side is horizon-friends' (`game`), registered by
//! horizon-official.
//!
//! It sees the game only through `horizon_hooks::share` (the engine's `ShareView`) and
//! `Hooks::lan_closed`, and keeps what it shares in its own static,
//! so it can leave the engine for horizon-friends (spec #14167). The screen and the
//! `horizon-ctl friends` command below still reach it from `Game` until then.
use super::Game;
use horizon_client::{Result, hud::HudBatch};
use horizon_friends::{Host, HostConfig, HostEvent, PathMode, host::DEFAULT_PORT, reach, stun};
use horizon_hooks::{
    Hooks,
    share::{LanOptions, ShareView},
};
use horizon_protocol::{GAME_VERSION, PROTOCOL_VERSION};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

mod screen;

use screen::{self as friends_screen, Action, FriendsView};
pub use screen::{TITLE, title};

struct Sharing {
    host: Host,
    /// Sharing opened the world to LAN, so stopping closes it again.
    opened_lan: bool,
    code: Option<String>,
    /// Wrong codes tried so far.
    wrong: u32,
    notice: Option<String>,
    copied: Option<Instant>,
    /// Sessions carried directly and through the relay so far.
    paths: (u32, u32),
}

/// The world shared, if one is. The game has one window player who can share (the engine
/// calls the hooks for it only), so one is all there is.
static SHARING: Mutex<Option<Sharing>> = Mutex::new(None);
/// Whether [`SHARING`] holds one: a tick with nothing shared is this one load.
static ON: AtomicBool = AtomicBool::new(false);

fn sharing() -> MutexGuard<'static, Option<Sharing>> {
    SHARING.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Ends the sharing, if there is one. The lock is let go before the caller touches the
/// game: closing LAN calls [`Hooks::lan_closed`], which takes it again.
fn take() -> Option<Sharing> {
    let taken = sharing().take();
    ON.store(false, Ordering::Relaxed);
    taken
}

/// Registered by the engine's entry while this module is part of it.
pub struct Friends;
pub static HOOKS: Friends = Friends;

/// No `Hooks::share` yet: the code and sessions are read while the screen is drawn or
/// `horizon-ctl friends status` asks, as before. The screen's page uses it (#14167 step 2a).
impl Hooks for Friends {
    /// The LAN server is closing: the code goes with it.
    fn lan_closed(&self) {
        if ON.load(Ordering::Relaxed)
            && let Some(mut sharing) = take()
        {
            sharing.host.stop();
        }
    }
}

pub const HELP: &[&str] = &[
    "friends host                   share the open world behind an invite code",
    "friends stop                   stop sharing",
    "friends status                 the code, how friends reach this host, the guests",
];

/// A string in the player's language, or the English one.
fn tr(world: &dyn ShareView, key: &str, english: &str) -> String {
    world.tr(key).unwrap_or_else(|| english.to_owned())
}

/// `HORIZON_STUN_SERVERS`, else `friendsStun` in horizon.txt, else the defaults.
fn stun_servers() -> Vec<String> {
    if std::env::var_os("HORIZON_STUN_SERVERS").is_some() {
        return stun::servers_from_env();
    }
    let dir = std::env::current_dir().unwrap_or_default();
    match horizon_client::ui_theme::preference(&dir, "friendsStun") {
        Some(list) => stun::parse_servers(&list),
        None => stun::servers_from_env(),
    }
}

fn env_flag(name: &str, default: bool) -> bool {
    std::env::var(name).map_or(default, |v| v != "0")
}

/// Start sharing the open world.
fn start(world: &mut dyn ShareView) -> std::result::Result<(), String> {
    if ON.load(Ordering::Relaxed) {
        return Ok(());
    }
    if !world.can_share() {
        return Err("open a singleplayer world first".into());
    }
    let (port, opened_lan) = match world.lan_port() {
        Some(port) => (port, false),
        // ShareToLanScreen's defaults, as the world options screen offers them.
        None => (world.open_lan(LanOptions::default())?, true),
    };
    let host = Host::start(HostConfig {
        lan: std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        protocol: PROTOCOL_VERSION,
        game: GAME_VERSION.to_owned(),
        relay: horizon_client::net::friends_relay_setting(),
        path: PathMode::from_env(),
        port: std::env::var("HORIZON_FRIENDS_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_PORT),
        reach: reach::Options {
            // HORIZON_FRIENDS_UPNP=0 leaves the router alone.
            map_port: env_flag("HORIZON_FRIENDS_UPNP", true),
            stun: stun_servers(),
            // Both ends on one machine (tests).
            loopback: env_flag("HORIZON_FRIENDS_LOOPBACK", false),
        },
    });
    let host = match host {
        Ok(h) => h,
        Err(e) => {
            if opened_lan {
                world.close_lan();
            }
            return Err(e.to_string());
        }
    };
    *sharing() = Some(Sharing {
        host,
        opened_lan,
        code: None,
        wrong: 0,
        notice: None,
        copied: None,
        paths: (0, 0),
    });
    ON.store(true, Ordering::Relaxed);
    Ok(())
}

/// Stop sharing: the code ends (direct port closed, router mapping
/// deleted); the world closes to LAN again if sharing opened it.
fn stop(world: &mut dyn ShareView) {
    if !world.can_share() {
        return;
    }
    let Some(mut sharing) = take() else {
        return;
    };
    sharing.host.stop();
    if sharing.opened_lan {
        world.close_lan();
    } else {
        let text = tr(
            world,
            "horizon.friends.stopped",
            "Stopped sharing the invite code.",
        );
        world.chat(&text);
    }
}

/// New codes and sessions, while the screen is drawn and when `friends status` asks.
fn tick(world: &mut dyn ShareView) {
    if !ON.load(Ordering::Relaxed) {
        return;
    }
    let mut announce = false;
    let mut stopped = None;
    {
        let mut guard = sharing();
        let Some(sharing) = guard.as_mut() else {
            return;
        };
        for event in sharing.host.poll() {
            match event {
                HostEvent::Code(code) => {
                    if sharing.code.is_some() && sharing.wrong >= horizon_friends::host::MAX_WRONG {
                        sharing.notice = Some(tr(
                            world,
                            "horizon.friends.short_code_changed",
                            "Someone kept trying a wrong short code, so it changed.",
                        ));
                        sharing.wrong = 0;
                    }
                    announce = sharing.code.is_none();
                    sharing.code = Some(code);
                }
                HostEvent::WrongCode { count } => {
                    sharing.wrong = count;
                    sharing.notice = Some(tr(
                        world,
                        "horizon.friends.wrong_code",
                        "Someone tried a wrong code.",
                    ));
                }
                HostEvent::Connected { direct: true, .. } => sharing.paths.0 += 1,
                HostEvent::Connected { direct: false, .. } => sharing.paths.1 += 1,
                HostEvent::Disconnected { .. } => {}
                HostEvent::RelayStopped => {
                    let text = tr(
                        world,
                        "horizon.friends.relay_stopped",
                        "Too many wrong short codes, so the relay is off until you share again. Friends can still join with the full code.",
                    );
                    sharing.notice = Some(text.clone());
                    stopped = Some(text);
                }
            }
        }
    }
    if let Some(text) = stopped {
        world.chat(&text);
    }
    if announce {
        let text = tr(
            world,
            "horizon.friends.code_ready",
            "Your invite code is ready: Game Menu, Play with Friends. Copy it and send it to your friends.",
        );
        world.chat(&text);
    }
}

fn copy(world: &mut dyn ShareView) {
    if !world.can_share() {
        return;
    }
    let Some(code) = sharing().as_ref().and_then(|s| s.code.clone()) else {
        return;
    };
    world.copy(&code);
    if let Some(s) = sharing().as_mut() {
        s.copied = Some(Instant::now());
    }
}

/// What Play with Friends shows.
fn status(world: &dyn ShareView) -> FriendsView {
    // Another server, or a splitscreen guest in P1's world: nothing to share.
    if !world.can_share() {
        return FriendsView::default();
    }
    let guard = sharing();
    let Some(s) = guard.as_ref() else {
        return FriendsView {
            shown: true,
            ..FriendsView::default()
        };
    };
    let mut guests = Vec::new();
    world.players(&mut |name| guests.push(name.to_owned()));
    let state = s.host.state();
    let mut status = if s.code.is_none() {
        tr(
            world,
            "horizon.friends.finding_route",
            "Finding how friends can reach you…",
        )
    } else {
        state.summary.clone()
    };
    if let Some(short) = &state.short {
        status.push_str(&format!(" Short code for the relay: {short}."));
    }
    if let Some(problem) = &state.relay_problem {
        status.push(' ');
        status.push_str(problem);
    }
    let reachable = matches!(
        state.verdict,
        Some(reach::Verdict::Mapped(_) | reach::Verdict::Public)
    ) || state.short.is_some();
    FriendsView {
        shown: true,
        sharing: true,
        code: s.code.clone(),
        status,
        reachable,
        help: (!reachable && s.code.is_some()).then(|| reach::HELP_URL.to_owned()),
        guests,
        notice: s.notice.clone(),
        copied: s
            .copied
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2)),
    }
}

/// `horizon-ctl friends host|stop|status`.
fn control(
    word: Option<&str>,
    world: &mut dyn ShareView,
) -> std::result::Result<serde_json::Value, String> {
    use serde_json::json;
    match word {
        Some("host") => {
            start(world)?;
            Ok(json!({ "ok": true, "lan_port": world.lan_port() }))
        }
        Some("stop") => {
            stop(world);
            Ok(json!({ "ok": true }))
        }
        Some("status") => {
            tick(world);
            let view = status(world);
            let guard = sharing();
            let sharing = guard.as_ref().filter(|_| view.sharing);
            let state = sharing.map(|s| s.host.state());
            Ok(json!({
                "sharing": view.sharing,
                "code": view.code,
                "short": state.as_ref().and_then(|s| s.short.as_ref().map(ToString::to_string)),
                "summary": view.status,
                "reachable": view.reachable,
                "verdict": state.as_ref().and_then(|s| s.verdict.as_ref().map(|v| format!("{v:?}"))),
                "direct_port": state.as_ref().map(|s| s.port),
                "sessions": sharing.map_or(0, |s| s.host.sessions()),
                "direct": sharing.map_or(0, |s| s.paths.0),
                "relayed": sharing.map_or(0, |s| s.paths.1),
                "guests": view.guests,
                "lan_port": world.lan_port(),
            }))
        }
        _ => Err("friends host|stop|status".into()),
    }
}

impl Game<'_> {
    /// Play with Friends. The world is paused under it in singleplayer, so
    /// new codes and sessions are read while it is drawn.
    pub(super) fn draw_friends_screen(&mut self, batch: &mut HudBatch) {
        let world = &mut self.share_view();
        tick(world);
        let view = status(world);
        horizon_client::widgets::begin_screen(|| {
            horizon_client::widgets::ScreenInfo::new("friends", friends_screen::TITLE)
        });
        friends_screen::draw(batch, &self.gui.atlas, self.cursor, &view, true);
    }

    pub(super) fn friends_screen_click(&mut self) {
        let view = status(&self.share_view());
        let Some(action) =
            friends_screen::click(&self.gui.atlas, self.gui_size(), self.cursor, &view)
        else {
            return;
        };
        self.audio.play_button_click();
        if action == Action::Done {
            self.screen = super::Screen::Paused;
            return;
        }
        let world = &mut self.share_view();
        match action {
            Action::Share => {
                if let Err(e) = start(world) {
                    let text = tr(
                        world,
                        "horizon.friends.share_failed",
                        "Couldn't share the world: %s",
                    )
                    .replacen("%s", &e, 1);
                    world.chat(&text);
                }
            }
            Action::Copy => copy(world),
            Action::Stop => stop(world),
            Action::Help => {
                if let Some(url) = &view.help {
                    world.open_url(url);
                }
            }
            Action::Done => {}
        }
    }

    /// Whether the pause menu offers Play with Friends (a singleplayer world).
    pub(super) fn friends_shown(&self) -> bool {
        self.can_share()
    }

    /// `horizon-ctl friends ...`.
    pub(super) fn friends_control(&mut self, words: &[&str]) -> Result<Option<serde_json::Value>> {
        if words[0] != "friends" {
            return Ok(None);
        }
        Ok(Some(control(
            words.get(1).copied(),
            &mut self.share_view(),
        )?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A world as the engine shows it, recording what the crate did to it.
    #[derive(Default)]
    struct Fake {
        can_share: bool,
        lan: Option<u16>,
        refuse_lan: bool,
        did: Vec<String>,
    }

    impl ShareView for Fake {
        fn can_share(&self) -> bool {
            self.can_share
        }
        fn lan_port(&self) -> Option<u16> {
            self.lan
        }
        fn open_lan(&mut self, options: LanOptions) -> std::result::Result<u16, String> {
            self.did.push(format!("open_lan {options:?}"));
            if self.refuse_lan {
                return Err("no free port".into());
            }
            self.lan = Some(25565);
            Ok(25565)
        }
        fn close_lan(&mut self) {
            self.did.push("close_lan".into());
            self.lan = None;
        }
        fn players(&self, each: &mut dyn FnMut(&str)) {
            each("Alex");
        }
        fn chat(&mut self, text: &str) {
            self.did.push(format!("chat {text}"));
        }
        fn copy(&mut self, text: &str) {
            self.did.push(format!("copy {text}"));
        }
        fn open_url(&mut self, url: &str) {
            self.did.push(format!("open_url {url}"));
        }
        fn tr(&self, _key: &str) -> Option<String> {
            None
        }
    }

    #[test]
    fn a_world_that_cannot_be_shared_is_left_alone() {
        // Another server, or a splitscreen guest in P1's world.
        let mut guest = Fake {
            lan: Some(25565),
            ..Fake::default()
        };
        assert_eq!(
            control(Some("host"), &mut guest).unwrap_err(),
            "open a singleplayer world first"
        );
        stop(&mut guest);
        copy(&mut guest);
        assert!(guest.did.is_empty(), "{:?}", guest.did);
        assert!(!status(&guest).shown);
        assert!(!ON.load(Ordering::Relaxed));
    }

    #[test]
    fn sharing_opens_lan_as_open_to_lan_does_and_a_refusal_shares_nothing() {
        let mut world = Fake {
            can_share: true,
            refuse_lan: true,
            ..Fake::default()
        };
        assert_eq!(start(&mut world).unwrap_err(), "no free port");
        assert_eq!(
            world.did,
            ["open_lan LanOptions { guest_commands: false, force_game_mode: true }"]
        );
        // Offered, not shared: the guests show once sharing does.
        let view = status(&world);
        assert!(view.shown && !view.sharing && view.guests.is_empty());
        assert!(!ON.load(Ordering::Relaxed));
    }
}
