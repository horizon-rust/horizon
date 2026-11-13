//! Share the open world over Tailcat: the pause menu's row opens this screen,
//! which opens the world to LAN (Open to LAN's defaults) if it isn't and runs
//! the `tailcat-for-minecraft` helper against that port (`horizon_client::tailcat`).
//! The helper answers with an `mcl1_...` invitation, which friends paste into
//! the multiplayer screen's Tailcat button. The world itself is the LAN one,
//! unchanged: tunnelled guests are LAN guests that reach the helper's loopback
//! listener.
use super::super::{Game, lan::LanSettings};
use horizon_client::{
    Result,
    hud::{HudAtlas, HudBatch, rgba},
    multiplayer::wrap,
    text::Text,
    ui,
};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The English title: the fallback of [`title`], and what the screen's widgets are named by.
pub const TITLE: &str = "Share with Tailcat";
fn tr(key: &str, english: &str) -> String {
    horizon_client::lang::get_or(key, english)
}
/// The title in the current language.
pub fn title() -> String {
    tr("horizon.tailcat.title", TITLE)
}

const WHITE: u32 = u32::MAX;
fn grey() -> u32 {
    rgba(160, 160, 160, 255)
}

type Hosted = std::result::Result<(horizon_client::tailcat::Running, String), String>;

pub const HELP: &[&str] = &[
    "tailcat host                   share the open world over Tailcat",
    "tailcat stop                   stop sharing",
    "tailcat status                 the invitation and whether the helper is ready",
];

/// The world is (or is becoming) shared over Tailcat.
pub(super) struct TailcatShare {
    /// The helper still starting; its result arrives on a worker thread.
    pending: Option<mpsc::Receiver<Hosted>>,
    running: Option<horizon_client::tailcat::Running>,
    invite: Option<String>,
    error: Option<String>,
    /// Sharing opened the world to LAN, so stopping closes it again.
    opened_lan: bool,
    copied: Option<Instant>,
}

impl TailcatShare {
    fn new(pending: mpsc::Receiver<Hosted>, opened_lan: bool) -> Self {
        Self {
            pending: Some(pending),
            running: None,
            invite: None,
            error: None,
            opened_lan,
            copied: None,
        }
    }

    fn failed(error: String) -> Self {
        Self {
            pending: None,
            running: None,
            invite: None,
            error: Some(error),
            opened_lan: false,
            copied: None,
        }
    }
}

/// What the screen shows.
#[derive(Clone, Debug, Default)]
pub struct TailcatView {
    /// A singleplayer world (not a server): sharing is offered.
    pub shown: bool,
    pub sharing: bool,
    pub invite: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub copied: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Share,
    Copy,
    Stop,
    Done,
}

impl Game<'_> {
    fn tailcat_share(&self) -> Option<&TailcatShare> {
        self.friends.as_ref().and_then(|s| s.tailcat.as_ref())
    }

    fn tailcat_share_mut(&mut self) -> Option<&mut TailcatShare> {
        self.friends.as_mut().and_then(|s| s.tailcat.as_mut())
    }

    fn set_tailcat(&mut self, share: TailcatShare) {
        match &mut self.friends {
            Some(s) => s.tailcat = Some(share),
            None => {
                self.friends = Some(super::Sharing {
                    friends: None,
                    tailcat: Some(share),
                });
            }
        }
    }

    /// Start sharing the open world over Tailcat, keeping an error in the
    /// screen's state so it can say what went wrong.
    pub(in crate::game) fn tailcat_start(&mut self) -> Result<()> {
        if self.tailcat_share().is_some() {
            return Ok(());
        }
        match self.tailcat_begin() {
            Ok(()) => Ok(()),
            Err(e) => {
                self.set_tailcat(TailcatShare::failed(e.to_string()));
                Err(e)
            }
        }
    }

    fn tailcat_begin(&mut self) -> Result<()> {
        self.friends_stop();
        if self.title.is_some() || (self.remote.is_some() && self.integrated.is_none()) {
            return Err("open a singleplayer world first".into());
        }
        let opened_lan = self.lan_port().is_none();
        if opened_lan {
            self.lan_open(LanSettings {
                port: horizon_client::lan::available_port(),
                guest_commands: false,
                force_game_mode: true,
            })?;
        }
        let port = self.lan_port().ok_or("the world isn't open to LAN")?;
        let (tx, rx) = mpsc::channel();
        let cache = horizon_client::logging::game_home().join("tailcat");
        std::thread::spawn(move || {
            let _ = tx.send(horizon_client::tailcat::host(port, &cache));
        });
        self.set_tailcat(TailcatShare::new(rx, opened_lan));
        Ok(())
    }

    /// Stop sharing: the helper stops and the world closes to LAN again if
    /// sharing opened it.
    pub(in crate::game) fn tailcat_stop(&mut self) {
        let Some(share) = self.friends.as_mut().and_then(|s| s.tailcat.take()) else {
            return;
        };
        let opened_lan = share.opened_lan;
        drop(share);
        if !self.friends.as_ref().is_some_and(|s| s.friends.is_some()) {
            self.friends = None;
        }
        if opened_lan {
            self.lan_close(true);
        } else {
            self.system_message(Text::literal(tr(
                "horizon.tailcat.stopped",
                "Stopped sharing over Tailcat.",
            )));
        }
    }

    /// The helper's answer, and whether it is still running.
    pub(super) fn tick_tailcat(&mut self) {
        let Some(share) = self.tailcat_share_mut() else {
            return;
        };
        if let Some(pending) = share.pending.take() {
            match pending.try_recv() {
                Ok(Ok((running, invite))) => {
                    share.running = Some(running);
                    share.invite = Some(invite);
                }
                Ok(Err(error)) => share.error = Some(error),
                Err(mpsc::TryRecvError::Empty) => share.pending = Some(pending),
                Err(mpsc::TryRecvError::Disconnected) => {
                    share.error = Some(tr(
                        "horizon.tailcat.failed",
                        "The Tailcat helper stopped before it was ready.",
                    ));
                }
            }
        }
        if let Some(running) = share.running.as_mut()
            && !running.is_alive()
            && share.invite.is_some()
        {
            share.error = Some(tr(
                "horizon.tailcat.stopped",
                "Stopped sharing over Tailcat.",
            ));
            share.running = None;
            share.invite = None;
        }
    }

    pub(super) fn tailcat_copy(&mut self) {
        let Some(invite) = self.tailcat_share().and_then(|s| s.invite.clone()) else {
            return;
        };
        use horizon_client::clipboard::Clipboard;
        self.clipboard.set(&invite);
        if let Some(share) = self.tailcat_share_mut() {
            share.copied = Some(Instant::now());
        }
    }

    /// Whether the pause menu offers the Tailcat row (a singleplayer world).
    pub(in crate::game) fn tailcat_shown(&self) -> bool {
        self.tailcat_view().shown
    }

    pub(super) fn tailcat_view(&self) -> TailcatView {
        let shown = self.remote.is_none() || self.integrated.is_some();
        let Some(share) = self.tailcat_share() else {
            return TailcatView {
                shown,
                ..TailcatView::default()
            };
        };
        let status = if let Some(error) = &share.error {
            error.clone()
        } else if share.invite.is_some() {
            tr(
                "horizon.tailcat.ready",
                "Friends can join with this invitation. Copy it and send it to them.",
            )
        } else {
            tr(
                "horizon.tailcat.starting",
                "Starting Tailcat and looking for a relay…",
            )
        };
        TailcatView {
            shown: true,
            sharing: share.invite.is_some() || share.pending.is_some() || share.running.is_some(),
            invite: share.invite.clone(),
            status,
            error: share.error.clone(),
            copied: share
                .copied
                .is_some_and(|t| t.elapsed() < Duration::from_secs(2)),
        }
    }

    /// Share with Tailcat. The world is paused under it in singleplayer, so
    /// the helper's answer is read while it is drawn.
    pub(super) fn draw_tailcat_screen(&mut self, batch: &mut HudBatch) {
        self.tick_tailcat();
        let view = self.tailcat_view();
        horizon_client::widgets::begin_screen(|| {
            horizon_client::widgets::ScreenInfo::new("tailcat", title())
        });
        draw(batch, &self.gui.atlas, self.cursor, &view);
    }

    pub(super) fn tailcat_screen_click(&mut self) {
        let view = self.tailcat_view();
        let Some(action) = click(&self.gui.atlas, self.gui_size(), self.cursor, &view) else {
            return;
        };
        self.audio.play_button_click();
        match action {
            Action::Share => {
                if let Err(e) = self.tailcat_start() {
                    self.system_message(Text::literal(
                        tr(
                            "horizon.tailcat.share_failed",
                            "Couldn't share the world: %s",
                        )
                        .replacen("%s", &e.to_string(), 1),
                    ));
                }
            }
            Action::Copy => self.tailcat_copy(),
            Action::Stop => self.tailcat_stop(),
            Action::Done => self.screen = super::super::Screen::Paused,
        }
    }

    /// `horizon-ctl tailcat ...`.
    pub(in crate::game) fn tailcat_control(
        &mut self,
        words: &[&str],
    ) -> Result<Option<serde_json::Value>> {
        use serde_json::json;
        if words.first() != Some(&"tailcat") {
            return Ok(None);
        }
        match words.get(1).copied() {
            Some("host") => {
                self.tailcat_start()?;
                Ok(Some(json!({ "ok": true, "lan_port": self.lan_port() })))
            }
            Some("stop") => {
                self.tailcat_stop();
                Ok(Some(json!({ "ok": true })))
            }
            Some("status") => {
                self.tick_tailcat();
                let view = self.tailcat_view();
                Ok(Some(json!({
                    "sharing": view.sharing,
                    "invite": view.invite,
                    "status": view.status,
                    "error": view.error,
                    "lan_port": self.lan_port(),
                })))
            }
            _ => Err("tailcat host|stop|status".into()),
        }
    }
}

struct Button {
    x: f32,
    y: f32,
    w: f32,
    label: String,
    action: Action,
    active: bool,
}

/// The invitation split into lines that fit `width`.
fn invite_lines(atlas: &HudAtlas, invite: &str, width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for ch in invite.chars() {
        let candidate = format!("{line}{ch}");
        if !line.is_empty() && HudBatch::text_width(atlas, &candidate) > width {
            lines.push(std::mem::take(&mut line));
            line.push(ch);
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn layout(
    atlas: &HudAtlas,
    (w, h): (f32, f32),
    view: &TailcatView,
) -> (Vec<(String, u32, f32)>, Vec<Button>) {
    let width = (w - 40.0).min(310.0);
    let cx = (w / 2.0).floor();
    let mut lines = Vec::new();
    let mut y = 40.0;
    let add = |lines: &mut Vec<(String, u32, f32)>, text: &str, color: u32, y: &mut f32| {
        for line in wrap(text, width, atlas) {
            lines.push((line, color, *y));
            *y += 10.0;
        }
    };
    let mut buttons = Vec::new();
    if view.sharing {
        lines.push((
            tr("horizon.multiplayer.invite_code", "Invite Code"),
            grey(),
            y,
        ));
        y += 12.0;
        match &view.invite {
            Some(invite) => {
                for line in invite_lines(atlas, invite, width) {
                    lines.push((line, WHITE, y));
                    y += 10.0;
                }
            }
            None => add(&mut lines, &view.status, WHITE, &mut y),
        }
        y += 6.0;
        buttons.push(Button {
            x: cx - 154.0,
            y,
            w: 150.0,
            label: if view.copied {
                tr("horizon.friends.copied", "Copied")
            } else {
                tr("horizon.friends.copy_code", "Copy Code")
            },
            action: Action::Copy,
            active: view.invite.is_some(),
        });
        buttons.push(Button {
            x: cx + 4.0,
            y,
            w: 150.0,
            label: tr("horizon.friends.stop_sharing", "Stop Sharing"),
            action: Action::Stop,
            active: true,
        });
        y += 30.0;
        if view.invite.is_some() {
            add(&mut lines, &view.status, grey(), &mut y);
        }
    } else {
        add(
            &mut lines,
            &tr(
                "horizon.tailcat.share_intro",
                "Share this world over Tailcat: it opens to LAN, and friends anywhere join with the invitation. No Tailscale account or port forwarding is needed.",
            ),
            WHITE,
            &mut y,
        );
        buttons.push(Button {
            x: cx - 100.0,
            y: y + 8.0,
            w: 200.0,
            label: tr("horizon.tailcat.start_sharing", "Start Sharing"),
            action: Action::Share,
            active: view.shown,
        });
        if let Some(error) = &view.error {
            y += 8.0;
            add(&mut lines, error, WHITE, &mut y);
        }
    }
    buttons.push(Button {
        x: cx - 100.0,
        y: h - 27.0,
        w: 200.0,
        label: "gui.done".to_owned(),
        action: Action::Done,
        active: true,
    });
    (lines, buttons)
}

fn label(key: &str) -> Text {
    if key.starts_with("gui.") {
        Text::translate(key, &[])
    } else {
        Text::literal(key)
    }
}

fn draw(batch: &mut HudBatch, atlas: &HudAtlas, cursor: (f32, f32), view: &TailcatView) {
    let (w, h) = (batch.width, batch.height);
    ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
    let title = Text::literal(title());
    let tw = atlas.font.width_text(&title);
    batch.text_styled(atlas, &title, ((w - tw) / 2.0).floor(), 15.0, WHITE, true);
    let (lines, buttons) = layout(atlas, (w, h), view);
    for (line, color, y) in lines {
        let lw = HudBatch::text_width(atlas, &line);
        batch.text(atlas, &line, ((w - lw) / 2.0).floor(), y, color, true);
    }
    for b in buttons {
        let hover = b.active && ui::hovered(cursor, b.x, b.y, b.w, 20.0);
        if matches!(b.action, Action::Done) {
            horizon_client::widgets::declare_action(
                horizon_client::widgets::Action::Done,
                [b.x, b.y, b.w, 20.0],
                b.active,
            );
        }
        ui::button(
            batch,
            atlas,
            b.x,
            b.y,
            b.w,
            20.0,
            &label(&b.label),
            b.active,
            hover,
        );
    }
}

fn click(
    atlas: &HudAtlas,
    size: (f32, f32),
    cursor: (f32, f32),
    view: &TailcatView,
) -> Option<Action> {
    layout(atlas, size, view)
        .1
        .into_iter()
        .find(|b| b.active && ui::hovered(cursor, b.x, b.y, b.w, 20.0))
        .map(|b| b.action)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atlas() -> HudAtlas {
        HudAtlas::build(std::path::Path::new("/nonexistent/horizon-test-assets")).unwrap()
    }

    #[test]
    fn share_copy_and_stop() {
        let atlas = atlas();
        let size = (427.0, 240.0);
        let idle = TailcatView {
            shown: true,
            ..TailcatView::default()
        };
        let (_, buttons) = layout(&atlas, size, &idle);
        let share = buttons.iter().find(|b| b.action == Action::Share).unwrap();
        assert_eq!(
            click(&atlas, size, (share.x + 1.0, share.y + 1.0), &idle),
            Some(Action::Share)
        );

        let starting = TailcatView {
            shown: true,
            sharing: true,
            status: "Starting Tailcat and looking for a relay…".into(),
            ..TailcatView::default()
        };
        let (_, buttons) = layout(&atlas, size, &starting);
        let copy = buttons.iter().find(|b| b.action == Action::Copy).unwrap();
        assert!(!copy.active, "nothing to copy before the invitation");

        let ready = TailcatView {
            invite: Some(format!("mcl1_{}", "A".repeat(180))),
            status: "Friends can join with this invitation.".into(),
            ..starting
        };
        let (lines, buttons) = layout(&atlas, size, &ready);
        assert!(
            lines.iter().all(|(_, _, y)| y + 9.0 <= size.1 - 27.0),
            "the invitation stays above the footer"
        );
        for action in [Action::Copy, Action::Stop, Action::Done] {
            let b = buttons.iter().find(|b| b.action == action).unwrap();
            assert_eq!(
                click(&atlas, size, (b.x + 1.0, b.y + 1.0), &ready),
                Some(action)
            );
        }
    }

    #[test]
    fn a_server_offers_no_sharing() {
        let atlas = atlas();
        let view = TailcatView::default();
        let (_, buttons) = layout(&atlas, (427.0, 240.0), &view);
        let share = buttons.iter().find(|b| b.action == Action::Share).unwrap();
        assert!(!share.active);
    }
}
