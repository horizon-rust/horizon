//! Splitscreen (Horizon Settings → Splitscreen..., off by default): players drop in and out of P1's world
//! on this computer, as on a console. P1 is the game as launched, with the keyboard, the mouse and
//! the first controller used in a menu. In a world, another controller pressing a button is
//! offered to join (several at once: each its own offer); pressing A adds a player, another `Game` in this process that joins P1's
//! server thread through memory, after it picks a profile and where to play in its view
//! (`split_profiles.rs`). Each player's pause menu has Splitscreen Settings: P1's Setup screen,
//! the others' profiles to switch to. A player who disconnects (pause menu) drops out; when P1
//! leaves the world, all do.
//!
//! Each player plays in a window: Window 1 (the game's), Window 2, 3... (opened when someone picks
//! New window, closed with its last player), or a monitor's (borderless over it). Players in one
//! window share it: halves for two (side by side or top and bottom), three one wide and two (or
//! in columns), quarters for four.
//! A joining player starts in Window 1 and moves with its controller's bumpers before it picks
//! its profile; the Setup screen (`split_ui.rs`) moves anyone later. Every player renders offscreen on one device and each
//! window's compositor copies its players' frames. Turning splitscreen on rebuilds P1's renderer
//! that way at once (the world is meshed again: a moment, in the menu); off, back.
//!
//! A controller that disconnects leaves its player waiting, its view dimmed: the same controller
//! reconnecting takes it back, A on another takes it over, B on another lets the player leave; a
//! player still picking a profile leaves at once. P1 keeps the keyboard and the mouse (a notice,
//! not a question).
//!
//! Keyboard and mouse drive the player whose view was clicked last (Esc frees the mouse, then
//! click another view); closing a player's own window drops that player. The players' frames are
//! paced once, at the fastest any wants.
//!
//! Design: one process, one `Game` per player, one wgpu device, one compositor per window. The
//! players share the game's data (`Resources`, the packs), one audio mixer, one controller reader
//! and P1's server. Not one game process per player in its own window: that needs GPU textures
//! shared across processes (per OS, unsafe), input, audio and the Setup screen across processes,
//! and a copy of everything per player. It was weighed and set aside (2026-10-27).
//!
//! `HORIZON_SPLIT=N` (testing): splitscreen on, and N-1 players without controllers join once P1
//! is in a world, as P2, P3... (`HORIZON_SPLIT_PICK=1`: picking profiles instead). The player in
//! seat N takes `horizon-ctl` on P1's socket path + "N".

use super::{
    Game, UiResources, build_renderer,
    pad::{PadEvent, PadId, PadRouter},
    start_game,
};
use horizon_client::split_layout::{self as layout, MAX_PLAYERS, Placement, Spot, View};
use horizon_client::{
    Result,
    gamepad::{Button, PadState},
    gfx::compositor::Compositor,
    hud::{HudBatch, rgba},
    widgets,
};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize},
    event::{DeviceEvent, DeviceId, ElementState, WindowEvent},
    event_loop::ActiveEventLoop,
    monitor::MonitorHandle,
    window::{Window, WindowId},
};

mod frames;
mod hud;
pub(super) mod launcher;
mod pads;
mod players;
mod setup;
mod windows;

pub(super) use hud::Hud;
use hud::*;
use pads::*;
use windows::*;

/// How long a controller's offer to join stays up.
const OFFER: Duration = Duration::from_secs(10);
/// The game's window, the first of the numbered ones.
const WINDOW_1: &str = "Window 1";
const NEW_WINDOW: &str = "New window";

thread_local! {
    /// The options file of the player whose turn it is (none for P1, whose is the game's own):
    /// every player runs on this thread, one after another.
    static OPTIONS: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// The `options.txt` of the player whose turn it is; `None` for P1.
pub(super) fn options_path() -> Option<PathBuf> {
    OPTIONS.with_borrow(Clone::clone)
}

struct Guest<'a> {
    game: Box<Game<'a>>,
    /// The profile it plays as; `None` while it picks one.
    name: Option<String>,
    /// Its seat (2 to `MAX_PLAYERS`): its number on screen (P2...), its control socket, its sounds.
    seat: usize,
    /// The window it plays in, by number.
    window: u8,
    pad: Option<PadId>,
    options: PathBuf,
    /// Its buttons and focus targets while another player's game runs.
    widgets: widgets::Saved,
    /// The bumpers it held at the last frame (LB, RB), while it chooses where to play.
    bumpers: [bool; 2],
    /// Its profile got into the world: turned away after that (a kick), it leaves; before, it
    /// picks again.
    entered: bool,
}

pub(super) struct Split<'a> {
    pub(super) host: Game<'a>,
    guests: Vec<Guest<'a>>,
    /// Splitscreen is on: every player renders through the compositors (else P1 draws on its
    /// window and every event goes straight to it).
    enabled: bool,
    /// `HORIZON_SPLIT=N`: players without controllers still to add once P1 is in a world.
    test_players: usize,
    pads: PadRouter,
    p1_pad: Option<PadId>,
    p1_window: u8,
    /// The windows players play in (Window 1, the game's, always first), each free or filling a
    /// monitor: what the setup screen's draft starts from (`layout::Draft`).
    windows: Vec<layout::Window>,
    /// Controllers offered to join, each since when: several may join at once.
    offers: Vec<(PadId, Instant)>,
    /// Pads whose buttons still count as the press that joined them: released first.
    fresh: Vec<PadId>,
    /// Players whose controller disconnected, longest waiting first.
    lost: Vec<Lost<PadId>>,
    /// `HORIZON_SPLIT_STATS`: when the frame times were last printed.
    stats: Option<Instant>,
    /// For the readout: where a frame's wall time goes (the turns with their renders, presents,
    /// the GPU wait, outside `about_to_wait`), summed since the last print, and when this frame
    /// started.
    spent: [Duration; 4],
    spent_frames: u32,
    frame_started: Option<Instant>,
    /// P1's name and when it was read: without `--name` reading it opens horizon-player.txt,
    /// too slow for every frame (the badge, the titles).
    p1_name: (String, Instant),
    /// The launcher's other Microsoft accounts' sessions, for splitscreen players to sign in
    /// with (`launcher::accounts`).
    accounts: Vec<std::sync::Arc<horizon_client::auth::AuthSession>>,
    /// The game's window and its compositor first, then monitors' and players' own.
    surfaces: Vec<Surface>,
    /// The views the players were last fitted to.
    applied: Vec<View>,
    /// Where each numbered window was when it closed, to open there again (this session).
    window_rects: std::collections::HashMap<u8, (LogicalPosition<f64>, LogicalSize<f64>)>,
    /// The monitors with their names, left to right, and the one Window 1 is on: read once a
    /// frame (`refresh_monitors`), the system's names are slow to ask for.
    screens: Vec<(String, MonitorHandle)>,
    main_screen: Option<String>,
    /// The seats the Setup screens last showed: a draft is by place in this list, so a change
    /// drops every draft.
    setup_seats: Vec<usize>,
    /// Who comes first in a window players share, by seat (1 = P1): set by the Setup screen and
    /// by where a joining player put itself with the bumpers. Seats it doesn't name come last.
    order: Vec<usize>,
    /// The player keyboard and mouse drive (0 = P1), and the pointer in the window it moved in.
    focus: usize,
    cursor: PhysicalPosition<f64>,
    limiter: horizon_client::window::FramerateLimiter,
    clock: horizon_client::window::SystemClock,
    /// P1's resources, shared by the other players' games for the process's lifetime.
    res: Option<&'static Arc<super::Resources>>,
    /// Whose game runs now (0 = P1), and P1's buttons while another's does.
    turn: usize,
    host_widgets: widgets::Saved,
}

impl<'a> Split<'a> {
    /// P1's game, with its compositor when it was made on the loader's surface.
    pub(super) fn new(mut host: Game<'a>, compositor: Option<Compositor>) -> Self {
        let test_players = std::env::var("HORIZON_SPLIT")
            .ok()
            .and_then(|n| n.parse::<usize>().ok())
            .map_or(0, |n| n.clamp(1, MAX_PLAYERS) - 1);
        if test_players > 0 {
            host.settings.extras.splitscreen = true;
        }
        let enabled = host.settings.extras.splitscreen;
        let p1_name = host.options.player_name();
        if enabled {
            host.options.offscreen = true;
            host.options.paced_by_split = true;
        }
        // Made after the window on an ordinary launch (no `resumed` follows).
        let surfaces = match (compositor, &host.window) {
            (Some(compositor), Some(window)) => vec![Surface {
                key: 1,
                placement: Placement::Free(None),
                window: window.clone(),
                compositor,
                size: window.inner_size(),
                focused: true,
                title: String::new(),
            }],
            _ => Vec::new(),
        };
        Self {
            host,
            guests: Vec::new(),
            enabled,
            test_players,
            pads: PadRouter::default(),
            p1_pad: None,
            p1_window: 1,
            windows: vec![layout::Window {
                number: 1,
                placement: Placement::Free(None),
            }],
            offers: Vec::new(),
            fresh: Vec::new(),
            lost: Vec::new(),
            stats: std::env::var_os("HORIZON_SPLIT_STATS").map(|_| Instant::now()),
            spent: Default::default(),
            spent_frames: 0,
            frame_started: None,
            p1_name: (p1_name, Instant::now()),
            accounts: launcher::accounts(),
            surfaces,
            applied: Vec::new(),
            window_rects: Default::default(),
            screens: Vec::new(),
            main_screen: None,
            setup_seats: Vec::new(),
            order: Vec::new(),
            focus: 0,
            cursor: PhysicalPosition::new(0.0, 0.0),
            limiter: horizon_client::window::FramerateLimiter::new(0),
            clock: Default::default(),
            res: None,
            turn: 0,
            host_widgets: Default::default(),
        }
    }

    fn players(&self) -> usize {
        1 + self.guests.len()
    }

    /// The first seat free (2 to `MAX_PLAYERS`).
    fn free_seat(&self) -> Option<usize> {
        (2..=MAX_PLAYERS).find(|n| self.guests.iter().all(|g| g.seat != *n))
    }

    /// The splitscreen folder in the game folder: `profiles/<name>/` holds each profile.
    fn folder() -> Result<PathBuf> {
        Ok(std::env::current_dir()?.join("splitscreen"))
    }

    /// Player `i`'s turn (0 = P1): its options file, and its own buttons and focus targets, which
    /// screens record while drawing and the control socket and the controller's cursor read.
    fn turn(&mut self, i: usize) {
        if let Some(game) = self.game(i) {
            horizon_client::hud::GUI_SCALE.store(
                game.settings.gui_scale,
                std::sync::atomic::Ordering::Relaxed,
            );
        }

        if i == self.turn {
            return;
        }
        let current = widgets::take();
        match self.turn {
            0 => self.host_widgets = current,
            t => {
                if let Some(g) = self.guests.get_mut(t - 1) {
                    g.widgets = current;
                }
            }
        }
        let next = match i {
            0 => std::mem::take(&mut self.host_widgets),
            _ => self
                .guests
                .get_mut(i - 1)
                .map(|g| std::mem::take(&mut g.widgets))
                .unwrap_or_default(),
        };
        widgets::put(next);
        OPTIONS.set(
            self.guests
                .get(i.wrapping_sub(1))
                .map(|g| g.options.clone()),
        );
        self.turn = i;
    }

    /// Player `i`'s game (0 = P1).
    fn game(&self, i: usize) -> Option<&Game<'a>> {
        match i {
            0 => Some(&self.host),
            _ => self.guests.get(i - 1).map(|g| &*g.game),
        }
    }

    fn game_mut(&mut self, i: usize) -> Option<&mut Game<'a>> {
        match i {
            0 => Some(&mut self.host),
            _ => self.guests.get_mut(i - 1).map(|g| &mut *g.game),
        }
    }

    /// Player `i`'s seat (P1's is 1) and the name players see ("New player" while it picks its
    /// profile). Screens show the name; the seat stays internal.
    fn who(&self, i: usize) -> (usize, String) {
        match i {
            0 => (1, self.p1_name.0.clone()),
            _ => {
                let g = &self.guests[i - 1];
                (
                    g.seat,
                    g.name.clone().unwrap_or_else(|| "New player".into()),
                )
            }
        }
    }

    /// Player `i`'s game, in its turn, and its window's id (a game only takes its own window's
    /// events).
    fn player(&mut self, i: usize) -> Option<(&mut dyn ApplicationHandler, WindowId)> {
        self.turn(i);
        let focus = self.focus;
        let game = self.game_mut(i)?;
        game.keyboard_mouse = i == focus;
        let id = game.window.as_ref()?.id();
        Some((game, id))
    }

    /// Give keyboard and mouse to one player, releasing the previous owner's held controls.
    fn handoff(&mut self, next: usize) {
        if next != self.focus {
            self.turn(self.focus);
            if let Some(game) = self.game_mut(self.focus) {
                for key in std::mem::take(&mut game.physical_keys) {
                    game.mod_key(key, false, false);
                    game.native_key_event(key, false, false);
                }
                for key in std::mem::take(&mut game.keys) {
                    if let Err(e) = game.mapped_key(key, false, false) {
                        log::warn!("Horizon: splitscreen: releasing input: {e}");
                    }
                }
                for button in std::mem::take(&mut game.physical_buttons) {
                    game.mod_mouse_key(button, false);
                    game.extras_mouse(button, false);
                }
                game.keys.clear();
                game.modifiers = Default::default();
                game.left = false;
                game.right = false;
                game.inventory_mouse_release(false);
                game.inventory_mouse_release(true);
                game.inventory_pick_release();
                game.loom_release();
                game.creative_screen.release();
                game.ui.options.release();
                game.dragging = false;
                game.stats_release();
                game.world_options_release();
                game.advancements_mouse(false, [0, 0]);
                if let Some(title) = &mut game.title {
                    title.release();
                    title.release_field();
                }
                game.capture(false);
                game.keyboard_mouse = false;
            }
        }
        self.focus = next;
        for i in 0..self.players() {
            if let Some(game) = self.game_mut(i) {
                game.keyboard_mouse = i == next;
            }
        }
    }

    fn captured(&self, i: usize) -> bool {
        self.game(i).is_some_and(|g| g.captured)
    }

    /// The pointer as the focused player sees it: from its view's corner.
    fn cursor_in_focus(&self) -> PhysicalPosition<f64> {
        let [x, y] = self.views().get(self.focus).map_or([0, 0], |v| v.1);
        let at = self.cursor;
        PhysicalPosition::new(at.x - f64::from(x), at.y - f64::from(y))
    }

    /// Splitscreen turned on: P1 renders offscreen on the same device and a compositor presents
    /// on its window; the world is meshed again for the new renderer.
    #[inline(never)]
    fn enable(&mut self) -> Result<()> {
        let window = self.host.window.clone().ok_or("no window")?;
        let mut old = self.host.renderer.take().ok_or("no renderer")?;
        old.release_surface();
        let api = self.host.settings.horizon.api;
        let built = build_renderer(
            window.clone(),
            true,
            &self.host.pack,
            &self.host.gui,
            Some(&old),
            None,
            api,
        );
        let renderer = match built {
            Ok(r) => r,
            Err(e) => {
                old.recreate_surface()?;
                self.host.renderer = Some(old);
                return Err(e);
            }
        };
        // A window it can't present on: P1 keeps drawing on it as before.
        let compositor = match old.compositor(window.clone()) {
            Ok(c) => c,
            Err(e) => {
                drop(renderer);
                old.recreate_surface()?;
                self.host.renderer = Some(old);
                return Err(e);
            }
        };
        self.host.renderer = Some(renderer);
        self.host.options.offscreen = true;
        self.host.options.paced_by_split = true;
        // The new renderer takes the old one's present mode, settings and all: P1's again.
        self.host.apply_settings();
        self.host.clear_lod();
        self.host.all_sections_changed();
        self.surfaces = vec![Surface {
            key: 1,
            placement: Placement::Free(None),
            size: window.inner_size(),
            window,
            compositor,
            focused: true,
            title: String::new(),
        }];
        self.enabled = true;
        Ok(())
    }

    /// Splitscreen turned off: the others leave, and P1 draws on its window again.
    #[inline(never)]
    fn disable(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        // P1's own buttons and focus targets back first: the others' go with them.
        for k in (0..self.guests.len()).rev() {
            self.remove_guest(k);
        }
        self.host.pad.feed = None;
        // Everything splitscreen kept, as when the game started: on again starts clean. P1's
        // controller stays P1's.
        self.host.split_hud = Default::default();
        self.offers.clear();
        self.fresh.clear();
        self.lost.clear();
        // Its own reader of the pads goes: nobody would drain its events (P1 reads its own).
        self.pads = PadRouter::default();
        self.p1_window = 1;
        self.windows = vec![layout::Window {
            number: 1,
            placement: Placement::Free(None),
        }];
        self.applied.clear();
        self.setup_seats.clear();
        self.order.clear();
        self.surfaces.truncate(1);
        let Some(main) = self.surfaces.pop() else {
            self.enabled = false;
            return Ok(());
        };
        main.window.set_visible(true);
        let old = self.host.renderer.take().ok_or("no renderer")?;
        old.take_back(main.compositor);
        let api = self.host.settings.horizon.api;
        let built = build_renderer(
            main.window.clone(),
            false,
            &self.host.pack,
            &self.host.gui,
            Some(&old),
            None,
            api,
        );
        let renderer = match built {
            Ok(r) => r,
            Err(e) => {
                // P1 stays offscreen, presented as before (alone now).
                let compositor = old.compositor(main.window.clone())?;
                self.host.renderer = Some(old);
                self.surfaces.push(Surface { compositor, ..main });
                return Err(e);
            }
        };
        drop(old);
        self.host.renderer = Some(renderer);
        self.host.window = Some(main.window.clone());
        self.host.options.offscreen = false;
        self.host.options.paced_by_split = false;
        // Alone again: the pointer is P1's, its view the whole window.
        self.focus = 0;
        self.host.keyboard_mouse = true;
        self.host.view_at = [0, 0];
        // From the offscreen renderer it took no vsync: P1's settings again.
        self.host.apply_settings();
        self.host.clear_lod();
        self.host.all_sections_changed();
        let id = main.window.id();
        self.host.window_event(
            event_loop,
            id,
            WindowEvent::Resized(main.window.inner_size()),
        );
        self.enabled = false;
        Ok(())
    }

    /// P1 is playing in a world others can join: its own (its server thread) or a server's.
    fn in_world(&self) -> bool {
        self.host.title.is_none()
            && self.host.player.is_some()
            && (self.host.integrated.is_some() || self.p1_server().is_some())
    }

    /// The server P1 joined over the network (a dedicated server, another computer's LAN
    /// world, a friend's invite): the others join it too, each as its own profile.
    fn p1_server(&self) -> Option<String> {
        if self.host.integrated.is_some() || self.host.remote.is_none() {
            return None;
        }
        self.host.server_address().map(str::to_owned)
    }
}

impl ApplicationHandler for Split<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.host.resumed(event_loop);
        if !self.enabled {
            return;
        }
        let (Some(window), Some(renderer)) = (&self.host.window, &self.host.renderer) else {
            return;
        };
        if self.surfaces.is_empty() {
            match renderer.compositor(window.clone()) {
                Ok(compositor) => self.surfaces.push(Surface {
                    key: 1,
                    placement: Placement::Free(None),
                    window: window.clone(),
                    compositor,
                    size: window.inner_size(),
                    focused: true,
                    title: String::new(),
                }),
                Err(e) => {
                    self.host.error = Some(e);
                    event_loop.exit();
                    return;
                }
            }
        }
        self.relayout(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if !self.enabled {
            return self.host.window_event(event_loop, id, event);
        }
        let Some(surface) = self.surfaces.iter().position(|s| s.window.id() == id) else {
            return;
        };
        match event {
            WindowEvent::Resized(size) => {
                let s = &mut self.surfaces[surface];
                s.size = size;
                s.compositor.resize(size.width, size.height);
                self.relayout(event_loop);
            }
            // The views draw themselves offscreen; the compositors present in about_to_wait.
            WindowEvent::RedrawRequested => {}
            // The game's window: P1's to handle (it saves the world, then quits); a player's game
            // would end the event loop before that. Another window: its players leave.
            WindowEvent::CloseRequested if surface == 0 => {
                if let Some((host, id)) = self.player(0) {
                    host.window_event(event_loop, id, WindowEvent::CloseRequested);
                }
            }
            WindowEvent::CloseRequested => {
                if let Err(e) = self.close_window(surface) {
                    log::warn!("Horizon: splitscreen: {e}");
                }
            }
            // Players' pads count while any of the windows has focus.
            WindowEvent::Focused(focused) => {
                self.surfaces[surface].focused = focused;
                let any = self.surfaces.iter().any(|s| s.focused);
                for i in 0..self.players() {
                    if let Some((player, id)) = self.player(i) {
                        player.window_event(event_loop, id, WindowEvent::Focused(any));
                    }
                }
            }
            WindowEvent::CursorMoved {
                device_id,
                position,
            } => {
                self.cursor = position;
                let position = self.cursor_in_focus();
                let focus = self.focus;
                if self.views().get(focus).is_some_and(|v| v.0 == surface)
                    && let Some((player, id)) = self.player(focus)
                {
                    player.window_event(
                        event_loop,
                        id,
                        WindowEvent::CursorMoved {
                            device_id,
                            position,
                        },
                    );
                }
            }
            WindowEvent::MouseInput {
                device_id,
                state: ElementState::Pressed,
                ..
            } => {
                // A click on another player's view gives it keyboard and mouse, unless the
                // focused player holds the mouse (playing): Esc frees it first, as in vanilla.
                let (x, y) = (self.cursor.x as u32, self.cursor.y as u32);
                let clicked = self.views().iter().position(|(s, [vx, vy], size)| {
                    *s == surface
                        && (*vx..vx + size.width).contains(&x)
                        && (*vy..vy + size.height).contains(&y)
                });
                if let Some(clicked) = clicked
                    && clicked != self.focus
                    && !self.captured(self.focus)
                {
                    self.handoff(clicked);
                    self.tag(clicked, Some("Keyboard and mouse"));
                    let position = self.cursor_in_focus();
                    if let Some((player, id)) = self.player(clicked) {
                        player.window_event(
                            event_loop,
                            id,
                            WindowEvent::CursorMoved {
                                device_id,
                                position,
                            },
                        );
                    }
                }
                let focus = self.focus;
                if let Some((player, id)) = self.player(focus) {
                    player.window_event(event_loop, id, event);
                }
            }
            _ => {
                let focus = self.focus;
                if let Some((player, id)) = self.player(focus) {
                    player.window_event(event_loop, id, event);
                }
            }
        }
        self.turn(0);
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        let focus = if self.enabled { self.focus } else { 0 };
        if let Some((player, _)) = self.player(focus) {
            player.device_event(event_loop, id, event);
        }
        self.turn(0);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.refresh_monitors();
        // The Splitscreen switch, turned on or off in P1's settings: applied at once.
        if self.host.settings.extras.splitscreen != self.enabled && self.host.window.is_some() {
            let switched = if self.enabled {
                self.disable(event_loop)
            } else {
                self.enable()
            };
            match switched {
                Ok(()) if self.enabled => self.relayout(event_loop),
                Ok(()) => {}
                Err(e) => {
                    log::warn!("Horizon: splitscreen could not switch: {e}");
                    self.host.settings.extras.splitscreen = self.enabled;
                }
            }
        }
        if !self.enabled {
            return self.host.about_to_wait(event_loop);
        }
        let started = Instant::now();
        if let Some(previous) = self.frame_started.replace(started) {
            // What the last frame left after its own work: events, the limiter, the OS.
            self.spent[3] += started.saturating_duration_since(previous);
        }
        if self.p1_name.1.elapsed() >= Duration::from_secs(1) {
            self.p1_name = (self.host.options.player_name(), Instant::now());
        }
        let pad_joins = self.route_pads();
        self.turn(0);
        self.host.about_to_wait(event_loop);
        let mut changed = self.drop_out();
        for pad in pad_joins {
            match self.join(Some(pad), None) {
                Ok(()) => changed = true,
                Err(e) => log::warn!("Horizon: splitscreen: a player could not join: {e}"),
            }
        }
        self.sync_profiles();
        changed |= self.pick_profiles();
        self.follow_pack_answer();
        // Testing: players without controllers, one per frame, once.
        if self.in_world() && self.test_players > 0 {
            // HORIZON_SPLIT_PICK: they pick a profile like a controller's player, else P2...
            let name = std::env::var_os("HORIZON_SPLIT_PICK")
                .is_none()
                .then(|| format!("P{}", self.free_seat().unwrap_or(0)));
            match self.join(None, name) {
                Ok(()) => {
                    self.test_players -= 1;
                    changed = true;
                }
                Err(e) => {
                    self.host.error = Some(e);
                    event_loop.exit();
                    return;
                }
            }
        }
        changed |= self.sync_setup();
        changed |= self.sync_surfaces(event_loop);
        // Also a new arrangement from the settings (a screen, Two on a screen).
        if changed || self.views() != self.applied {
            self.relayout(event_loop);
        }
        // Badges while others play too.
        // Off in P1's Setup (Names): a badge only for a moment, to say where.
        let together = self.players() > 1;
        let names = self.host.settings.extras.splitscreen_names;
        self.host.split_hud.badge = together.then(|| self.p1_name.0.clone());
        self.host.split_hud.names = names;
        for g in &mut self.guests {
            g.game.split_hud.badge = Some(g.name.clone().unwrap_or_else(|| "New player".into()));
            g.game.split_hud.names = names;
        }
        let master = self.host.audio.master;
        // The pause menu's Splitscreen Settings while another player plays here (joined: a
        // profile picked, in the world), not for P1 alone; Horizon Settings > Splitscreen... is
        // always there.
        let joined = self.guests.iter().any(|g| g.name.is_some());
        self.host.split_hud.menu = joined;
        for i in 0..self.guests.len() {
            self.turn(i + 1);
            // One set of speakers: P1's volume.
            self.guests[i].game.audio.master = master;
            self.guests[i].game.about_to_wait(event_loop);
            // A player's game stops the event loop on an error, as P1's does: say why.
            if let Some(e) = &self.guests[i].game.error {
                log::error!("Horizon: splitscreen: seat {}: {e}", self.guests[i].seat);
            }
            // Choosing its profile, the bumpers move its view (its pad as its game read it: the
            // controller, or `horizon-ctl pad`).
            let place = self.stop_label(i + 1);
            let game = &mut self.guests[i].game;
            game.split_hud.menu = true;
            if game.ui.options.page() == horizon_client::options_ui::Page::SplitProfiles
                && game.screen == super::Screen::Options
            {
                game.ui.options.split_profiles.place = place;
                let state = game.pad.state;
                let held = [
                    state.held(Button::LeftBumper),
                    state.held(Button::RightBumper),
                ];
                let before = std::mem::replace(&mut self.guests[i].bumpers, held);
                if held[0] && !before[0] {
                    self.move_with_bumpers(i + 1, true);
                } else if held[1] && !before[1] {
                    self.move_with_bumpers(i + 1, false);
                }
            }
        }
        self.turn(0);
        let rendered = Instant::now();
        // Each window shows its players' frames. Only the game's waits for the display (vsync):
        // windows each waiting would divide the rate.
        let views = self.views();
        let renderers: Vec<_> = std::iter::once(&self.host.renderer)
            .chain(self.guests.iter().map(|g| &g.game.renderer))
            .collect();
        let vsync = self.host.settings.vsync;
        for (k, surface) in self.surfaces.iter_mut().enumerate() {
            let frames: Vec<_> = renderers
                .iter()
                .zip(&views)
                .filter(|(_, v)| v.0 == k)
                .filter_map(|(r, v)| Some((r.as_ref()?.offscreen_frame()?, v.1)))
                .collect();
            surface.compositor.set_vsync(vsync && k == 0);
            surface.compositor.present(&frames);
        }
        let presented = Instant::now();
        // The players' GPU work waits here, once every player has submitted: one player's CPU
        // work overlapped the GPU drawing another's.
        let renderers = std::iter::once(&mut self.host.renderer)
            .chain(self.guests.iter_mut().map(|g| &mut g.game.renderer));
        for renderer in renderers.flatten() {
            renderer.paced_elsewhere = true;
            renderer.wait_in_flight();
        }
        let waited = Instant::now();
        for (k, d) in [rendered - started, presented - rendered, waited - presented]
            .into_iter()
            .enumerate()
        {
            self.spent[k] += d;
        }
        // The rest of the frame (the limiter, the OS) counts as outside, from `waited` on.
        self.spent[3] = self.spent[3].saturating_sub(waited - started);
        self.spent_frames += 1;
        self.print_stats();
        // One pace for all: the fastest any player wants, so an idle one (vanilla's AFK limit)
        // can't slow the others. With vsync the display paces anything at or above its refresh
        // rate; the limiter on top would miss refreshes.
        let limit = std::iter::once(self.host.framerate_limit())
            .chain(self.guests.iter().map(|g| g.game.framerate_limit()))
            .max()
            .unwrap_or(60);
        let refresh_hz = self
            .surfaces
            .first()
            .and_then(|s| s.window.current_monitor()?.refresh_rate_millihertz())
            .map_or(60, |mhz| mhz.div_ceil(1000) as i32);
        if !(vsync && limit >= refresh_hz) {
            horizon_client::window::end_frame(&mut self.limiter, limit, &mut self.clock);
        }
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        for i in (0..self.players()).rev() {
            if let Some((player, _)) = self.player(i) {
                player.exiting(event_loop);
            }
        }
        self.turn(0);
    }
}
