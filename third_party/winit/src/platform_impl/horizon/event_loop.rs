use std::cell::Cell;
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use horizon_switch::{Libnx, Pointer, TouchScreen};

use super::{DeviceId, MonitorHandle, PlatformSpecificEventLoopAttributes, WindowId};
use crate::dpi::{PhysicalPosition, PhysicalSize};
use crate::error::EventLoopError;
use crate::event::{
    self, ElementState, MouseButton, MouseScrollDelta, StartCause, TouchPhase, WindowEvent,
};
use crate::event_loop::{self, ControlFlow, DeviceEvents};
use crate::window::{
    CustomCursor as RootCustomCursor, CustomCursorSource, Theme, WindowId as RootWindowId,
};

/// The longest the loop sleeps between looks at HID, the applet and the proxy: the Switch has no
/// event to wait on, and a thread must never spin (a busy one starves lower-priority threads).
const POLL: Duration = Duration::from_millis(4);

pub struct EventLoop<T> {
    window_target: event_loop::ActiveEventLoop,
    user_events_sender: mpsc::Sender<T>,
    user_events_receiver: mpsc::Receiver<T>,
}

impl<T: 'static> EventLoop<T> {
    pub(crate) fn new(_: &PlatformSpecificEventLoopAttributes) -> Result<Self, EventLoopError> {
        let (user_events_sender, user_events_receiver) = mpsc::channel();
        Ok(Self {
            window_target: event_loop::ActiveEventLoop {
                p: ActiveEventLoop {
                    control_flow: Cell::new(ControlFlow::default()),
                    exit: Cell::new(false),
                    redraw: Arc::new(AtomicBool::new(false)),
                    window_created: Cell::new(false),
                },
                _marker: PhantomData,
            },
            user_events_sender,
            user_events_receiver,
        })
    }

    pub fn run<F>(self, mut handler: F) -> Result<(), EventLoopError>
    where
        F: FnMut(event::Event<T>, &event_loop::ActiveEventLoop),
    {
        let target = &self.window_target;
        let id = RootWindowId(WindowId);
        let mut touch = TouchScreen::new();
        let mut cause = StartCause::Init;
        let mut size = super::screen_size();
        let mut focused = false;
        let mut cursor: Option<PhysicalPosition<f64>> = None;
        let mut button_down = false;
        let mut closing = false;

        loop {
            handler(event::Event::NewEvents(cause), target);
            if cause == StartCause::Init {
                handler(event::Event::Resumed, target);
            }

            // The window was created in a callback: tell the app its size, as other backends do.
            if target.p.window_created.replace(false) {
                size = super::screen_size();
                handler(
                    event::Event::WindowEvent { window_id: id, event: WindowEvent::Resized(size) },
                    target,
                );
            }

            // Docking or undocking changes the screen's size.
            let now_size = super::screen_size();
            if now_size != size {
                size = now_size;
                handler(
                    event::Event::WindowEvent { window_id: id, event: WindowEvent::Resized(size) },
                    target,
                );
            }

            // HOME and the home menu: focus changes, and a request to exit.
            let in_focus = Libnx::in_focus();
            if in_focus != focused {
                focused = in_focus;
                handler(
                    event::Event::WindowEvent { window_id: id, event: WindowEvent::Focused(focused) },
                    target,
                );
            }
            if !Libnx::main_loop() && !closing {
                closing = true;
                handler(
                    event::Event::WindowEvent { window_id: id, event: WindowEvent::CloseRequested },
                    target,
                );
                // An app that ignores the request is still asked to leave on this pass.
                target.p.exit.set(true);
            }

            for pointer_event in
                pointer_events(touch.poll(), size, &mut cursor, &mut button_down)
            {
                handler(event::Event::WindowEvent { window_id: id, event: pointer_event }, target);
            }

            while let Ok(user) = self.user_events_receiver.try_recv() {
                handler(event::Event::UserEvent(user), target);
            }

            if target.p.redraw.swap(false, Ordering::Relaxed) {
                handler(
                    event::Event::WindowEvent { window_id: id, event: WindowEvent::RedrawRequested },
                    target,
                );
            }

            handler(event::Event::AboutToWait, target);

            if target.p.exiting() {
                break;
            }

            let start = Instant::now();
            let requested_resume = match target.p.control_flow() {
                ControlFlow::Poll => {
                    cause = StartCause::Poll;
                    // Still give other threads the core.
                    std::thread::yield_now();
                    continue;
                },
                ControlFlow::Wait => None,
                ControlFlow::WaitUntil(instant) => Some(instant),
            };
            // HID has no wake-up: sleep one short step, then look again.
            if !target.p.redraw.load(Ordering::Relaxed) {
                let left = requested_resume.map(|t| t.saturating_duration_since(start));
                std::thread::sleep(left.map_or(POLL, |d| d.min(POLL)));
            }
            cause = match requested_resume {
                Some(requested_resume) if Instant::now() >= requested_resume => {
                    StartCause::ResumeTimeReached { start, requested_resume }
                },
                _ => StartCause::WaitCancelled { start, requested_resume },
            };
        }

        handler(event::Event::LoopExiting, target);
        Ok(())
    }

    pub fn window_target(&self) -> &event_loop::ActiveEventLoop {
        &self.window_target
    }

    pub fn create_proxy(&self) -> EventLoopProxy<T> {
        EventLoopProxy { user_events_sender: self.user_events_sender.clone() }
    }
}

/// The touch screen as a mouse: the finger moves the cursor and holds the left button down, a
/// second finger scrolls, and tapping it is a right click.
fn pointer_events(
    pointer: Pointer,
    size: PhysicalSize<u32>,
    cursor: &mut Option<PhysicalPosition<f64>>,
    button_down: &mut bool,
) -> Vec<WindowEvent> {
    let device_id = event::DeviceId(DeviceId);
    let mut out = Vec::new();
    if let Some([x, y]) = pointer.at {
        let at = PhysicalPosition::new(x as f64 * size.width as f64, y as f64 * size.height as f64);
        if *cursor != Some(at) {
            *cursor = Some(at);
            out.push(WindowEvent::CursorMoved { device_id, position: at });
        }
    }
    if pointer.down != *button_down {
        *button_down = pointer.down;
        out.push(WindowEvent::MouseInput {
            device_id,
            state: if pointer.down { ElementState::Pressed } else { ElementState::Released },
            button: MouseButton::Left,
        });
    }
    if pointer.right_tap {
        for state in [ElementState::Pressed, ElementState::Released] {
            out.push(WindowEvent::MouseInput { device_id, state, button: MouseButton::Right });
        }
    }
    if pointer.scroll != 0.0 {
        out.push(WindowEvent::MouseWheel {
            device_id,
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, pointer.scroll as f64)),
            phase: TouchPhase::Moved,
        });
    }
    out
}

pub struct EventLoopProxy<T: 'static> {
    user_events_sender: mpsc::Sender<T>,
}

impl<T> EventLoopProxy<T> {
    pub fn send_event(&self, event: T) -> Result<(), event_loop::EventLoopClosed<T>> {
        self.user_events_sender
            .send(event)
            .map_err(|mpsc::SendError(x)| event_loop::EventLoopClosed(x))
    }
}

impl<T> Clone for EventLoopProxy<T> {
    fn clone(&self) -> Self {
        Self { user_events_sender: self.user_events_sender.clone() }
    }
}

impl<T> Unpin for EventLoopProxy<T> {}

pub struct ActiveEventLoop {
    control_flow: Cell<ControlFlow>,
    exit: Cell<bool>,
    pub(super) redraw: Arc<AtomicBool>,
    pub(super) window_created: Cell<bool>,
}

impl ActiveEventLoop {
    pub fn create_custom_cursor(&self, source: CustomCursorSource) -> RootCustomCursor {
        let _ = source.inner;
        RootCustomCursor { inner: super::PlatformCustomCursor }
    }

    pub fn primary_monitor(&self) -> Option<MonitorHandle> {
        Some(MonitorHandle)
    }

    pub fn available_monitors(&self) -> VecDeque<MonitorHandle> {
        VecDeque::from([MonitorHandle])
    }

    #[inline]
    pub fn listen_device_events(&self, _allowed: DeviceEvents) {}

    #[inline]
    pub fn system_theme(&self) -> Option<Theme> {
        None
    }

    #[cfg(feature = "rwh_06")]
    #[inline]
    pub fn raw_display_handle_rwh_06(
        &self,
    ) -> Result<rwh_06::RawDisplayHandle, rwh_06::HandleError> {
        Ok(rwh_06::RawDisplayHandle::Horizon(rwh_06::HorizonDisplayHandle::new()))
    }

    pub fn set_control_flow(&self, control_flow: ControlFlow) {
        self.control_flow.set(control_flow)
    }

    pub fn control_flow(&self) -> ControlFlow {
        self.control_flow.get()
    }

    pub(crate) fn exit(&self) {
        self.exit.set(true);
    }

    pub(crate) fn exiting(&self) -> bool {
        self.exit.get()
    }

    pub(crate) fn owned_display_handle(&self) -> OwnedDisplayHandle {
        OwnedDisplayHandle
    }
}

#[derive(Clone)]
pub(crate) struct OwnedDisplayHandle;

impl OwnedDisplayHandle {
    #[cfg(feature = "rwh_06")]
    #[inline]
    pub fn raw_display_handle_rwh_06(
        &self,
    ) -> Result<rwh_06::RawDisplayHandle, rwh_06::HandleError> {
        Ok(rwh_06::HorizonDisplayHandle::new().into())
    }
}
