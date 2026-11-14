//! Play with friends, host side: "Start Sharing" on the Play with Friends
//! page (`PAGE`, what the pause menu's Friends icon opens) opens the world to LAN if
//! it isn't (with Open to LAN's defaults) and shares that port behind an
//! invite code (horizon-friends). The code holds this computer's addresses
//! and the key, so friends join with no server of ours; a relay the player
//! configured adds a short code and a fallback. The integrated server is the
//! LAN one, unchanged: tunnelled guests are LAN guests that connect from the
//! loopback address. The joining side is horizon-friends' (`game`), registered by
//! horizon-official.
//!
//! It sees the game only through `horizon_hooks` (the `ShareView` its `Hooks::share` tick
//! gets, `Hooks::lan_closed`, its page, `tr`) and keeps what it shares in its own statics, so
//! it can leave the engine for horizon-friends (spec #14167). The `horizon-ctl friends` command
//! below still reaches it from `Game` until then.
use super::Game;
use horizon_client::Result;
use horizon_friends::{Host, HostConfig, HostEvent, PathMode, host::DEFAULT_PORT, reach, stun};
use horizon_hooks::{
    Hooks,
    pages::{Act, Page, Place, Row},
    share::{LanOptions, ShareView},
};
use horizon_protocol::{GAME_VERSION, PROTOCOL_VERSION};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

/// The English title: the fallback of [`title`], and what the screen's widgets are named by.
pub const TITLE: &str = "Play with Friends";
/// The title in the current language.
pub fn title() -> String {
    tr("horizon.friends.title", TITLE)
}

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

/// What the page left for the next [`Hooks::share`] tick, which has the game's view.
#[derive(Default)]
struct PageState {
    /// Chat lines (new codes, the relay stopping), said in order.
    chat: Vec<String>,
    /// A button pressed on the page.
    pressed: Option<Action>,
    /// The page was drawn since the last tick: its guests are read again.
    open: bool,
    /// The other players, as the last tick while the page was open read them.
    guests: Vec<String>,
    /// The action of each row the page last showed.
    acts: Vec<Option<Action>>,
}
static PAGE_STATE: Mutex<PageState> = Mutex::new(PageState {
    chat: Vec::new(),
    pressed: None,
    open: false,
    guests: Vec::new(),
    acts: Vec::new(),
});
/// Whether [`PAGE_STATE`] has something for the tick.
static WORK: AtomicBool = AtomicBool::new(false);

fn page() -> MutexGuard<'static, PageState> {
    PAGE_STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A button of the page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Share,
    Copy,
    Stop,
    /// Open the troubleshooting page.
    Help,
}

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

impl Hooks for Friends {
    /// What the page left for the game: chat lines, a button's press, the guests while it is
    /// open. With nothing left, one relaxed load.
    fn share(&self, world: &mut dyn ShareView) {
        if !WORK.swap(false, Ordering::Relaxed) {
            return;
        }
        let (lines, pressed, open) = {
            let mut page = page();
            let open = std::mem::take(&mut page.open);
            (std::mem::take(&mut page.chat), page.pressed.take(), open)
        };
        for line in lines {
            world.chat(&line);
        }
        if let Some(action) = pressed {
            press(action, world);
        }
        if open {
            let mut guests = Vec::new();
            world.players(&mut |name| guests.push(name.to_owned()));
            page().guests = guests;
        }
    }

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
fn tr(key: &str, english: &str) -> String {
    horizon_hooks::tr(key).unwrap_or_else(|| english.to_owned())
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
            "horizon.friends.stopped",
            "Stopped sharing the invite code.",
        );
        world.chat(&text);
    }
}

/// New codes and sessions, while the page is drawn and when `friends status` asks. Its chat
/// lines wait for the next tick ([`Hooks::share`]), or for [`say`].
fn poll() {
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
                        "horizon.friends.wrong_code",
                        "Someone tried a wrong code.",
                    ));
                }
                HostEvent::Connected { direct: true, .. } => sharing.paths.0 += 1,
                HostEvent::Connected { direct: false, .. } => sharing.paths.1 += 1,
                HostEvent::Disconnected { .. } => {}
                HostEvent::RelayStopped => {
                    let text = tr(
                        "horizon.friends.relay_stopped",
                        "Too many wrong short codes, so the relay is off until you share again. Friends can still join with the full code.",
                    );
                    sharing.notice = Some(text.clone());
                    stopped = Some(text);
                }
            }
        }
    }
    let mut page = page();
    page.chat.extend(stopped);
    if announce {
        page.chat.push(tr(
            "horizon.friends.code_ready",
            "Your invite code is ready: Game Menu, Play with Friends. Copy it and send it to your friends.",
        ));
    }
    if !page.chat.is_empty() {
        WORK.store(true, Ordering::Relaxed);
    }
}

/// The chat lines waiting, said now.
fn say(world: &mut dyn ShareView) {
    let lines = std::mem::take(&mut page().chat);
    for line in lines {
        world.chat(&line);
    }
}

/// A button of the page, on the tick after its press.
fn press(action: Action, world: &mut dyn ShareView) {
    match action {
        Action::Share => {
            if let Err(e) = start(world) {
                let text = tr(
                    "horizon.friends.share_failed",
                    "Couldn't share the world: %s",
                )
                .replacen("%s", &e, 1);
                world.chat(&text);
            }
        }
        Action::Copy => copy(world),
        Action::Stop => stop(world),
        // The system browser, as the folder buttons open their folders.
        Action::Help => world.open_url(reach::HELP_URL),
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
#[derive(Clone, Debug, Default)]
struct View {
    sharing: bool,
    code: Option<String>,
    /// Who can join and what to do when friends on the internet can't (or,
    /// before the code, what the game is doing).
    status: String,
    /// Friends on the internet can reach the host.
    reachable: bool,
    /// The players in the world besides the host.
    guests: Vec<String>,
    notice: Option<String>,
    /// The code was just copied.
    copied: bool,
}

/// What Play with Friends shows; `guests` is asked only while sharing.
fn view(guests: impl FnOnce() -> Vec<String>) -> View {
    let guard = sharing();
    let Some(s) = guard.as_ref() else {
        return View::default();
    };
    let state = s.host.state();
    let mut status = if s.code.is_none() {
        tr(
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
    View {
        sharing: true,
        code: s.code.clone(),
        status,
        reachable,
        guests: guests(),
        notice: s.notice.clone(),
        copied: s
            .copied
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2)),
    }
}

/// The page's rows for `view`, each with the button's action: the layout today's Friends
/// screen had (the engine draws it, `game/page_screen.rs`).
fn rows(view: &View, rows: &mut Vec<Row>, acts: &mut Vec<Option<Action>>) {
    let mut row = |row: Row, act: Option<Action>| {
        rows.push(row);
        acts.push(act);
    };
    let hint = || {
        tr(
            "horizon.friends.hint",
            "Friends paste the code after the title screen's Friends button, in Direct Connection, or on the launcher's Play page.",
        )
    };
    if !view.sharing {
        row(
            Row::text(tr(
                "horizon.friends.share_intro",
                "Share this world behind an invite code: it opens to LAN, and friends on the internet join with the code. No server of ours is involved.",
            )),
            None,
        );
        row(Row::Space(4), None);
        row(Row::text(hint()).quiet(), None);
        row(Row::Space(8), None);
        row(
            Row::button("horizon.friends.start_sharing"),
            Some(Action::Share),
        );
        return;
    }
    row(
        Row::text(tr("horizon.multiplayer.invite_code", "Invite Code")).quiet(),
        None,
    );
    row(Row::Space(2), None);
    match &view.code {
        Some(code) => row(Row::text(code.clone()).groups(), None),
        None => row(Row::text(view.status.clone()), None),
    }
    row(Row::Space(6), None);
    let copy = if view.copied {
        "horizon.friends.copied"
    } else {
        "horizon.friends.copy_code"
    };
    row(
        Row::button(copy).half().active(view.code.is_some()),
        Some(Action::Copy),
    );
    row(
        Row::button("horizon.friends.stop_sharing").half(),
        Some(Action::Stop),
    );
    if view.code.is_some() {
        // Who can join: grey when friends anywhere can, white (it asks
        // for something) when only the LAN and VPNs can.
        let status = Row::text(view.status.clone());
        row(
            if view.reachable {
                status.quiet()
            } else {
                status
            },
            None,
        );
        row(Row::Space(4), None);
    }
    let here = if view.guests.is_empty() {
        tr("horizon.friends.none_yet", "No friends here yet.")
    } else {
        tr("horizon.friends.here_now", "Here now: %s").replacen("%s", &view.guests.join(", "), 1)
    };
    row(Row::text(here).quiet(), None);
    if let Some(notice) = &view.notice {
        row(Row::text(notice.clone()), None);
    }
    // Where friends paste the code, while it fits above the footer.
    row(Row::text(hint()).quiet(), None);
    // Troubleshooting beside Done when friends on the internet can't reach the host.
    if !view.reachable && view.code.is_some() {
        row(
            Row::button("horizon.friends.troubleshooting").footer(),
            Some(Action::Help),
        );
    }
}

/// Play with Friends' page: what the pause menu's Friends icon opens. Its rows are read when
/// it is drawn; a press waits for the next tick, which has the game's view.
pub struct FriendsPage;
pub static PAGE: FriendsPage = FriendsPage;

impl Page for FriendsPage {
    fn id(&self) -> &'static str {
        "horizon_friends.share"
    }
    fn title(&self) -> &'static str {
        "horizon.friends.title"
    }
    fn place(&self) -> Place {
        Place::PauseFriends
    }
    fn rows(&self, out: &mut Vec<Row>) {
        // The world is paused under the page in singleplayer, so new codes and sessions are
        // read while it is drawn.
        poll();
        let guests = std::mem::take(&mut page().guests);
        let view = view(|| guests);
        let mut acts = Vec::new();
        rows(&view, out, &mut acts);
        let mut page = page();
        page.guests = view.guests;
        page.acts = acts;
        page.open = true;
        WORK.store(true, Ordering::Relaxed);
    }
    fn act(&self, row: usize, act: Act) {
        let mut page = page();
        if act == Act::Press
            && let Some(Some(action)) = page.acts.get(row).copied()
        {
            page.pressed = Some(action);
            WORK.store(true, Ordering::Relaxed);
        }
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
            poll();
            say(world);
            let view = if world.can_share() {
                view(|| {
                    let mut guests = Vec::new();
                    world.players(&mut |name| guests.push(name.to_owned()));
                    guests
                })
            } else {
                View::default()
            };
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
        // Offered, not shared: the guests aren't asked for.
        let view = view(|| unreachable!("the guests of a world not shared"));
        assert!(!view.sharing && view.guests.is_empty());
        assert!(!ON.load(Ordering::Relaxed));
    }

    #[test]
    fn the_page_shows_todays_screen_and_a_press_runs_on_the_next_tick() {
        // Sharing, with a code friends on the internet can't reach yet.
        let shared = View {
            sharing: true,
            code: Some("K7QM2-AAAAA".into()),
            status: "Friends on your network can join.".into(),
            ..View::default()
        };
        let (mut out, mut acts) = (Vec::new(), Vec::new());
        rows(&shared, &mut out, &mut acts);
        let buttons: Vec<_> = out
            .iter()
            .zip(&acts)
            .filter_map(|(row, act)| match row {
                Row::Button {
                    label,
                    active,
                    half,
                    footer,
                    ..
                } => Some((*label, *active, *half, *footer, *act)),
                _ => None,
            })
            .collect();
        assert_eq!(
            buttons,
            [
                (
                    "horizon.friends.copy_code",
                    true,
                    true,
                    false,
                    Some(Action::Copy)
                ),
                (
                    "horizon.friends.stop_sharing",
                    true,
                    true,
                    false,
                    Some(Action::Stop)
                ),
                (
                    "horizon.friends.troubleshooting",
                    true,
                    false,
                    true,
                    Some(Action::Help)
                ),
            ]
        );
        assert!(out.iter().any(
            |row| matches!(row, Row::Text { text, groups: true, .. } if text == "K7QM2-AAAAA")
        ));
        // Before the code there is nothing to copy.
        let (mut out, mut acts) = (Vec::new(), Vec::new());
        rows(
            &View {
                code: None,
                ..shared
            },
            &mut out,
            &mut acts,
        );
        assert!(out.iter().any(|row| matches!(
            row,
            Row::Button {
                label: "horizon.friends.copy_code",
                active: false,
                ..
            }
        )));

        // Not sharing: Start Sharing. Its press waits for the tick, which has the world.
        let (mut out, mut acts) = (Vec::new(), Vec::new());
        rows(&View::default(), &mut out, &mut acts);
        let share = acts.iter().position(|a| *a == Some(Action::Share)).unwrap();
        page().acts = acts;
        PAGE.act(share, Act::Press);
        let mut world = Fake {
            can_share: true,
            refuse_lan: true,
            ..Fake::default()
        };
        HOOKS.share(&mut world);
        assert_eq!(
            world.did,
            [
                "open_lan LanOptions { guest_commands: false, force_game_mode: true }",
                "chat Couldn't share the world: no free port",
            ]
        );
        // Pressed once, run once.
        HOOKS.share(&mut world);
        assert_eq!(world.did.len(), 2);
    }
}
