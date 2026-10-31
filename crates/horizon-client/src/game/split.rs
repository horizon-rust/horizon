//! Splitscreen (in parts; #3541): more players on this computer in P1's world, each its own `Game`
//! in this process that joins P1's server thread through memory. Every player renders offscreen
//! on one device and the window's compositor copies their frames into their views (halves for
//! two, `split_layout`). Keyboard and mouse drive the player whose view was clicked last (Esc
//! frees the mouse first). A player who leaves its world (pause menu, Disconnect) drops out;
//! when P1 leaves, all do. Alone, P1 plays exactly as without splitscreen: no compositor.
//!
//! Design: one process, one `Game` per player, one wgpu device. The players share the game's
//! data (`Resources`, the packs), one audio output and P1's server. Not one game process per
//! player: that needs GPU textures, input and audio shared across processes.
//!
//! `HORIZON_SPLIT=N` (testing, the only way in so far): N-1 players join once P1 is in its
//! world, as P2, P3... The player in seat N takes `horizon-ctl` on P1's socket path + "N".

use super::{
    Game, UiResources, build_renderer,
    pad::{PadId, PadRouter},
    start_game,
};
use horizon_client::split_layout::{self as layout, MAX_PLAYERS, View};
use horizon_client::{Result, gfx::compositor::Compositor, widgets};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use winit::{
    application::ApplicationHandler,
    dpi::{PhysicalPosition, PhysicalSize},
    event::{DeviceEvent, DeviceId, ElementState, WindowEvent},
    event_loop::ActiveEventLoop,
    window::WindowId,
};

mod pads;
mod players;

thread_local! {
    /// The options file of the player whose turn it is (none for P1, whose is the game's own):
    /// every player runs on this thread, one after another.
    static OPTIONS: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// The `options.txt` of the player whose turn it is; `None` for P1.
pub(super) fn options_path() -> Option<PathBuf> {
    OPTIONS.with_borrow(Clone::clone)
}

impl Game<'_> {
    /// An error stops the event loop, as ever, but not in another splitscreen player's game:
    /// that player drops out (`Split::drop_out`), the others play on.
    pub(in crate::game) fn exit_on_error(&self, event_loop: &ActiveEventLoop) {
        if !self.options.split_guest {
            event_loop.exit();
        }
    }
}

struct Guest<'a> {
    game: Box<Game<'a>>,
    /// The name it plays as.
    name: String,
    /// Its seat (2 to `MAX_PLAYERS`): its number (P2...), its control socket, its sounds.
    seat: usize,
    options: PathBuf,
    /// The controller it plays with.
    pad: Option<PadId>,
    /// Its buttons and focus targets while another player's game runs.
    widgets: widgets::Saved,
}

pub(super) struct Split<'a> {
    pub(super) host: Game<'a>,
    guests: Vec<Guest<'a>>,
    /// `HORIZON_SPLIT=N`: players still to add once P1 is in a world.
    test_players: usize,
    /// P1's window's compositor, while another player is in (else P1 draws on its window).
    compositor: Option<Compositor>,
    size: PhysicalSize<u32>,
    /// The views the players were last fitted to.
    applied: Vec<View>,
    /// The player keyboard and mouse drive (0 = P1), and the pointer in the window.
    focus: usize,
    cursor: PhysicalPosition<f64>,
    limiter: horizon_client::window::FramerateLimiter,
    clock: horizon_client::window::SystemClock,
    /// P1's resources, shared by the other players' games for the process's lifetime.
    res: Option<&'static Arc<super::Resources>>,
    /// Whose game runs now (0 = P1), and P1's buttons while another's does.
    turn: usize,
    host_widgets: widgets::Saved,
    /// The controllers while several play (`pads`), and P1's.
    pads: PadRouter,
    p1_pad: Option<PadId>,
    /// Pads whose buttons still count as the press that gave them: released first.
    fresh: Vec<PadId>,
}

impl<'a> Split<'a> {
    pub(super) fn new(host: Game<'a>) -> Self {
        let test_players = std::env::var("HORIZON_SPLIT")
            .ok()
            .and_then(|n| n.parse::<usize>().ok())
            .map_or(0, |n| n.clamp(1, MAX_PLAYERS) - 1);
        Self {
            host,
            guests: Vec::new(),
            test_players,
            compositor: None,
            size: PhysicalSize::new(1, 1),
            applied: Vec::new(),
            focus: 0,
            cursor: PhysicalPosition::new(0.0, 0.0),
            limiter: horizon_client::window::FramerateLimiter::new(0),
            clock: Default::default(),
            res: None,
            turn: 0,
            host_widgets: Default::default(),
            pads: Default::default(),
            p1_pad: None,
            fresh: Vec::new(),
        }
    }

    fn players(&self) -> usize {
        1 + self.guests.len()
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

    /// Player `i`'s game, in its turn, and its window's id.
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
                    game.native_key_event(key, false, false);
                }
                for key in std::mem::take(&mut game.keys) {
                    if let Err(e) = game.mapped_key(key, false, false) {
                        log::warn!("Horizon: splitscreen: releasing input: {e}");
                    }
                }
                for button in std::mem::take(&mut game.physical_buttons) {
                    game.extras_mouse(button, false);
                }
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

    /// The pointer as the focused player sees it: from its view's corner.
    fn cursor_in_focus(&self) -> PhysicalPosition<f64> {
        let [x, y] = self.views().get(self.focus).map_or([0, 0], |v| v.1);
        let at = self.cursor;
        PhysicalPosition::new(at.x - f64::from(x), at.y - f64::from(y))
    }

    /// Every player's view in the window.
    fn views(&self) -> Vec<View> {
        layout::place(&vec![0; self.players()], &[self.size], 0, 0)
    }

    /// Every player into its view: its corner, then its size.
    fn relayout(&mut self, event_loop: &ActiveEventLoop) {
        self.applied = self.views();
        for (i, (_, at, size)) in self.applied.clone().into_iter().enumerate() {
            if let Some(game) = self.game_mut(i) {
                game.view_at = at;
            }
            if let Some((player, id)) = self.player(i) {
                player.window_event(event_loop, id, WindowEvent::Resized(size));
            }
        }
        self.turn(0);
    }

    /// Another player joins: P1 renders offscreen on the same device and a compositor presents
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
        self.size = window.inner_size();
        self.compositor = Some(compositor);
        Ok(())
    }

    /// P1 alone again: it draws on its window again.
    #[inline(never)]
    fn disable(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        for k in (0..self.guests.len()).rev() {
            self.remove_guest(k);
        }
        self.stop_routing();
        self.applied.clear();
        let Some(compositor) = self.compositor.take() else {
            return Ok(());
        };
        let window = self.host.window.clone().ok_or("no window")?;
        let old = self.host.renderer.take().ok_or("no renderer")?;
        old.take_back(compositor);
        let api = self.host.settings.horizon.api;
        let built = build_renderer(
            window.clone(),
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
                self.compositor = Some(old.compositor(window)?);
                self.host.renderer = Some(old);
                return Err(e);
            }
        };
        drop(old);
        self.host.renderer = Some(renderer);
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
        let id = window.id();
        self.host
            .window_event(event_loop, id, WindowEvent::Resized(window.inner_size()));
        Ok(())
    }

    /// The testing players (`HORIZON_SPLIT`) join, one a frame, once P1 is in its own world:
    /// whether one did.
    fn admit(&mut self, event_loop: &ActiveEventLoop) -> bool {
        let in_world = self.host.title.is_none()
            && self.host.player.is_some()
            && self.host.integrated.is_some();
        if !in_world || self.test_players == 0 {
            return false;
        }
        match self.join() {
            Ok(()) => {
                self.test_players -= 1;
                true
            }
            Err(e) => {
                self.host.error = Some(e);
                event_loop.exit();
                false
            }
        }
    }

    /// Each player's frame into its view, then the GPU wait, once, for all of them.
    fn present(&mut self) {
        let views = self.views();
        let Some(compositor) = &mut self.compositor else {
            return;
        };
        let frames: Vec<_> = std::iter::once(&self.host.renderer)
            .chain(self.guests.iter().map(|g| &g.game.renderer))
            .zip(&views)
            .filter_map(|(r, v)| Some((r.as_ref()?.offscreen_frame()?, v.1)))
            .collect();
        compositor.set_vsync(self.host.settings.vsync);
        compositor.present(&frames);
        // One player's CPU work overlapped the GPU drawing another's.
        let renderers = std::iter::once(&mut self.host.renderer)
            .chain(self.guests.iter_mut().map(|g| &mut g.game.renderer));
        for renderer in renderers.flatten() {
            renderer.paced_elsewhere = true;
            renderer.wait_in_flight();
        }
    }
}

impl ApplicationHandler for Split<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.host.resumed(event_loop);
        if self.compositor.is_some() {
            self.relayout(event_loop);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.compositor.is_none() {
            return self.host.window_event(event_loop, id, event);
        }
        match event {
            WindowEvent::Resized(size) => {
                self.size = size;
                if let Some(c) = &mut self.compositor {
                    c.resize(size.width, size.height);
                }
                self.relayout(event_loop);
            }
            // The views draw themselves offscreen; the compositor presents in about_to_wait.
            WindowEvent::RedrawRequested => {}
            // P1's to handle (it saves the world, then quits).
            WindowEvent::CloseRequested => {
                if let Some((host, id)) = self.player(0) {
                    host.window_event(event_loop, id, WindowEvent::CloseRequested);
                }
            }
            WindowEvent::Focused(focused) => {
                for i in 0..self.players() {
                    if let Some((player, id)) = self.player(i) {
                        player.window_event(event_loop, id, WindowEvent::Focused(focused));
                    }
                }
            }
            WindowEvent::CursorMoved {
                device_id,
                position,
            } => {
                self.cursor = position;
                let position = self.cursor_in_focus();
                if let Some((player, id)) = self.player(self.focus) {
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
                let clicked = self.views().iter().position(|(_, [vx, vy], size)| {
                    (*vx..vx + size.width).contains(&x) && (*vy..vy + size.height).contains(&y)
                });
                let holding = self.game(self.focus).is_some_and(|g| g.captured);
                if let Some(clicked) = clicked
                    && clicked != self.focus
                    && !holding
                {
                    self.handoff(clicked);
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
                if let Some((player, id)) = self.player(self.focus) {
                    player.window_event(event_loop, id, event);
                }
            }
            _ => {
                if let Some((player, id)) = self.player(self.focus) {
                    player.window_event(event_loop, id, event);
                }
            }
        }
        self.turn(0);
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        // Alone: exactly P1's path, as in window_event (no GUI-scale store per mouse motion).
        if self.compositor.is_none() {
            return self.host.device_event(event_loop, id, event);
        }
        if let Some((player, _)) = self.player(self.focus) {
            player.device_event(event_loop, id, event);
        }
        self.turn(0);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // The last other player gone: P1 draws on its window again.
        if self.compositor.is_some()
            && self.guests.is_empty()
            && let Err(e) = self.disable(event_loop)
        {
            log::warn!("Horizon: splitscreen could not switch back: {e}");
        }
        if self.compositor.is_none() {
            self.host.about_to_wait(event_loop);
            if self.admit(event_loop) {
                self.relayout(event_loop);
            }
            return;
        }
        self.route_pads();
        self.turn(0);
        self.host.about_to_wait(event_loop);
        let changed = self.drop_out() | self.admit(event_loop);
        if changed || self.views() != self.applied {
            self.relayout(event_loop);
        }
        let master = self.host.audio.master;
        for i in 0..self.guests.len() {
            self.turn(i + 1);
            // One set of speakers: P1's volume.
            self.guests[i].game.audio.master = master;
            self.guests[i].game.about_to_wait(event_loop);
            // A player's game stops the event loop on an error, as P1's does: say why.
            if let Some(e) = &self.guests[i].game.error {
                log::error!("Horizon: splitscreen: P{}: {e}", self.guests[i].seat);
            }
        }
        self.turn(0);
        self.present();
        // One pace for all: the fastest any player wants, so an idle one (vanilla's AFK limit)
        // can't slow the others. With vsync the display paces anything at or above its refresh
        // rate; the limiter on top would miss refreshes.
        let limit = std::iter::once(self.host.framerate_limit())
            .chain(self.guests.iter().map(|g| g.game.framerate_limit()))
            .max()
            .unwrap_or(60);
        let refresh_hz = self
            .host
            .window
            .as_ref()
            .and_then(|w| w.current_monitor()?.refresh_rate_millihertz())
            .map_or(60, |mhz| mhz.div_ceil(1000) as i32);
        if !(self.host.settings.vsync && limit >= refresh_hz) {
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
