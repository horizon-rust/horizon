//! A game started without its prepared files (the game build run by hand, not
//! through the launcher): a window that says where to get the launcher
//! (`horizon_client::no_resources`) and quits when it is closed. No loader, no
//! crash report: nothing went wrong in the game.
use super::*;
use horizon_client::no_resources::{Action, DOWNLOAD_URL, NoResources};
use std::sync::atomic::AtomicBool;
use winit::event::MouseButton;

struct Missing {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    gui: UiResources,
    screen: NoResources,
    /// The pointer in physical pixels.
    pointer: (f64, f64),
    api: horizon_client::graphics::GraphicsApi,
    error: Option<Box<dyn std::error::Error>>,
    /// Set to end the screen (the launcher's Quit and Start).
    exit: Arc<AtomicBool>,
    /// Android has the window: nothing is drawn (and no redraws asked for) until `resumed`.
    suspended: bool,
    /// The system keyboard is allowed (a dialog wants text).
    ime_on: bool,
    /// The mobile build's launcher (sign in, download), in place of the notice.
    launcher: Option<super::mobile_launcher::Host>,
    /// The launcher page's scroll in dp, and the furthest it can go.
    scroll: f32,
    max_scroll: f32,
    /// A finger down on the launcher: where, the scroll then, whether it became a scroll.
    drag: Option<((f64, f64), f32, bool)>,
}

/// Shows the screen until the player closes it.
pub(super) fn show(looked_in: &std::path::Path, settings: &Settings) -> Result<()> {
    let mut app = Missing {
        window: None,
        renderer: None,
        gui: UiResources {
            atlas: HudAtlas::build(looked_in)?,
            panorama: Arc::new(horizon_client::gfx::panorama::Faces {
                size: 1,
                pixels: [0u8; 4].repeat(6),
            }),
        },
        screen: NoResources::new(looked_in),
        pointer: (0.0, 0.0),
        api: settings.horizon.api,
        error: None,
        exit: Arc::new(AtomicBool::new(false)),
        suspended: false,
        ime_on: false,
        launcher: None,
        scroll: 0.0,
        max_scroll: 0.0,
        drag: None,
    };
    horizon_client::hud::GUI_SCALE.store(settings.gui_scale, Ordering::Relaxed);
    super::new_event_loop()?.run_app(&mut app)?;
    app.error.map_or(Ok(()), Err)
}

impl Missing {
    /// Physical pixels per dp, as the Ui works it out.
    fn density(&self) -> f32 {
        let [w, h] = self.size();
        (w as f32 / 1040.0).min(h as f32 / 700.0).clamp(1.0, 4.0)
    }

    fn size(&self) -> [u32; 2] {
        self.renderer
            .as_ref()
            .map_or([1280, 720], |r| [r.config.width, r.config.height])
    }

    /// `f` over a Ui of the window's size, the cursor where the pointer is.
    fn with_ui<R>(
        &self,
        batch: &mut HudBatch,
        f: impl FnOnce(&mut horizon_client::ui_theme::Ui) -> R,
    ) -> R {
        let cursor = (
            self.pointer.0 as f32 / batch.scale,
            self.pointer.1 as f32 / batch.scale,
        );
        let mut ui = horizon_client::ui_theme::Ui::new(&self.gui.atlas, batch, cursor);
        let result = f(&mut ui);
        ui.finish();
        result
    }

    fn draw(&mut self) {
        if self.suspended {
            return;
        }
        self.sync_ime();
        let mut batch = HudBatch::new(self.size());
        if let Some(host) = &self.launcher {
            let view = host.view(self.scroll);
            let max = self.with_ui(&mut batch, |ui| {
                let frame = horizon_client::mobile_launcher::layout(ui, &view);
                horizon_client::mobile_launcher::draw(ui, &frame);
                frame.max_scroll()
            });
            self.max_scroll = max;
            self.scroll = self.scroll.clamp(0.0, max);
        } else {
            self.with_ui(&mut batch, |ui| self.screen.draw(ui));
        }
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        match renderer.render(&loading_frame(&batch)) {
            Ok(true) => return,
            Ok(false) => {}
            Err(e) => log::warn!("Horizon: frame failed: {e}"),
        }
        // The frame was not shown (the first one often isn't): the glyphs it
        // rasterized never reached the atlas, so draw them again next frame.
        horizon_client::ui_theme::reset_atlas();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }

    /// A text dialog is open over the launcher.
    fn dialog_open(&self) -> bool {
        self.launcher.as_ref().is_some_and(|h| h.dialog_active())
    }

    /// The system keyboard is allowed only while a dialog wants text.
    fn sync_ime(&mut self) {
        let want = self.dialog_open();
        if want != self.ime_on
            && let Some(window) = &self.window
        {
            window.set_ime_allowed(want);
            self.ime_on = want;
        }
    }

    fn click(&mut self, event_loop: &ActiveEventLoop) {
        let mut batch = HudBatch::new(self.size());
        if let Some(host) = self.launcher.clone() {
            let view = host.view(self.scroll);
            let cmd = self.with_ui(&mut batch, |ui| {
                let frame = horizon_client::mobile_launcher::layout(ui, &view);
                horizon_client::mobile_launcher::click(ui, &frame)
            });
            if let Some(cmd) = cmd {
                // Start ends the screen from here; a worker can't reach the loop.
                let exit = self.exit.clone();
                host.command(cmd, &move || exit.store(true, Ordering::Relaxed));
                self.scroll = if matches!(cmd, horizon_client::mobile_launcher::Cmd::Tab(_)) {
                    0.0
                } else {
                    self.scroll
                };
            }
            return;
        }
        match self.with_ui(&mut batch, |ui| self.screen.click(ui)) {
            Some(Action::Download) => packs::open_uri(DOWNLOAD_URL),
            Some(Action::Quit) => event_loop.exit(),
            None => {}
        }
    }
}

impl ApplicationHandler for Missing {
    fn suspended(&mut self, _: &ActiveEventLoop) {
        // Android: the window is gone until `resumed`.
        self.suspended = true;
        if let Some(renderer) = &mut self.renderer {
            renderer.suspend();
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            // Back from the background: a new swapchain for the same window.
            self.suspended = false;
            if let Some(renderer) = &mut self.renderer
                && let Err(e) = renderer.resume()
            {
                log::warn!("Horizon: could not bring the window back: {e}");
            }
            if let Some(window) = &self.window {
                window.request_redraw();
            }
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Horizon")
            .with_window_icon(horizon_client::window::icon())
            .with_inner_size(winit::dpi::LogicalSize::new(1040.0, 700.0));
        let made = event_loop
            .create_window(attributes)
            .map_err(Into::into)
            .and_then(|window| {
                let window = Arc::new(window);
                let renderer =
                    loading_renderer(window.clone(), false, Some(&self.gui), None, self.api)?;
                Ok((window, renderer))
            });
        match made {
            Ok((window, renderer)) => {
                window.request_redraw();
                if super::mobile_launcher::wanted() && self.launcher.is_none() {
                    let for_wake = window.clone();
                    let folder = std::env::current_dir().unwrap_or_else(|_| ".".into());
                    self.launcher = Some(super::mobile_launcher::Host::new(
                        folder,
                        Arc::new(move || for_wake.request_redraw()),
                    ));
                }
                self.window = Some(window);
                self.renderer = Some(renderer);
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(window) = self.window.clone().filter(|w| w.id() == id) else {
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = &mut self.renderer {
                    r.resize(size.width, size.height);
                }
                window.request_redraw();
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = (position.x, position.y);
                window.request_redraw();
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => self.click(event_loop),
            // A finger: a drag scrolls the page; a touch that stays put is a click where it lifts.
            WindowEvent::Touch(touch) => {
                use winit::event::TouchPhase;
                let at = (touch.location.x, touch.location.y);
                self.pointer = at;
                let density = self.density();
                match touch.phase {
                    TouchPhase::Started => self.drag = Some((at, self.scroll, false)),
                    TouchPhase::Moved => {
                        if let Some((start, from, dragging)) = self.drag {
                            let dy = (at.1 - start.1) as f32 / density;
                            if dragging || dy.abs() > 10.0 {
                                self.scroll = (from - dy).clamp(0.0, self.max_scroll);
                                self.drag = Some((start, from, true));
                            }
                        }
                    }
                    TouchPhase::Ended => {
                        if let Some((_, _, dragging)) = self.drag.take() {
                            if !dragging {
                                self.click(event_loop);
                            }
                        }
                    }
                    TouchPhase::Cancelled => self.drag = None,
                }
                window.request_redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y * 48.0,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32 / self.density(),
                };
                self.scroll = (self.scroll - dy).clamp(0.0, self.max_scroll);
                window.request_redraw();
            }
            // A dialog (naming an instance) takes typed text, from the system keyboard or keys.
            WindowEvent::Ime(winit::event::Ime::Commit(text)) if self.dialog_open() => {
                if let Some(h) = &self.launcher {
                    h.dialog_type(Some(&text), false);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if self.dialog_open() => {
                if event.state == ElementState::Pressed
                    && let Some(h) = &self.launcher
                {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::Backspace) => h.dialog_type(None, true),
                        PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter) => {
                            let exit = self.exit.clone();
                            h.command(horizon_client::mobile_launcher::Cmd::DialogOk, &move || {
                                exit.store(true, Ordering::Relaxed)
                            });
                        }
                        PhysicalKey::Code(KeyCode::Escape) => {
                            let exit = self.exit.clone();
                            h.command(
                                horizon_client::mobile_launcher::Cmd::DialogCancel,
                                &move || exit.store(true, Ordering::Relaxed),
                            );
                        }
                        _ => {
                            if let Some(t) = event
                                .text
                                .as_deref()
                                .filter(|t| t.chars().all(|c| !c.is_control()))
                            {
                                h.dialog_type(Some(t), false);
                            }
                        }
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::Escape) =>
            {
                event_loop.exit()
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if horizon_threads::stop_requests() > 0 {
            event_loop.exit();
            return;
        }
        if self.exit.load(Ordering::Relaxed) {
            event_loop.exit();
            return;
        }
        // The sky moves: a frame twice a second while the launcher is showing.
        if self.launcher.is_some() && !self.suspended {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        // Wake now and then for Ctrl+C in the console; redraws come with events.
        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(if self.launcher.is_some() { 500 } else { 250 }),
        ));
    }
}
