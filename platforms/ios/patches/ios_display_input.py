#!/usr/bin/env python3
"""iOS-only viewport and touch pointer adaptation applied to the Actions checkout.

Horizon gameplay stays upstream-owned. Do not edit the tracked game crates.
This runs after the existing Nova diagnostics and deep-stack callback patches.
The iOS UIKit window should use device bounds, not desktop logical 1280x720.
Native winit touch events are converted into its existing mouse event path,
so title, options and world screens receive normal click/drag semantics.
"""
from pathlib import Path


def change(path: str, before: str, after: str) -> None:
    file = Path(path)
    source = file.read_text(encoding="utf-8")
    assert source.count(before) == 1, (
        f"{path}: upstream structure changed near {before[:80]!r}"
    )
    file.write_text(source.replace(before, after, 1), encoding="utf-8")


window_attributes = """        #[cfg(target_os = "ios")]
        let attributes = {
            use winit::platform::ios::{
                ScreenEdge, ValidOrientations, WindowAttributesExtIOS,
            };
            attributes
                .with_prefers_status_bar_hidden(true)
                .with_prefers_home_indicator_hidden(true)
                .with_preferred_screen_edges_deferring_system_gestures(ScreenEdge::ALL)
                .with_valid_orientations(ValidOrientations::Landscape)
        };
"""
attributes_anchor = """        let attributes =
            horizon_client::window::with_icon(Window::default_attributes().with_title("Horizon"));
"""
for name in ("loading_screen.rs", "window_handler.rs"):
    change(
        f"crates/horizon-client/src/game/{name}",
        attributes_anchor,
        attributes_anchor + window_attributes,
    )

loading = "crates/horizon-client/src/game/loading_screen.rs"
change(
    loading,
    """        let window = match event_loop.create_window(
            attributes
                .with_inner_size(winit::dpi::LogicalSize::new(size[0], size[1]))
                .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None)))
                .with_visible(!self.options.as_ref().is_some_and(|o| o.offscreen)),
        ) {""",
    """        // UIKit owns the iOS window bounds: desktop --window-size creates a
        // 16:9 Metal surface on a 4:3 iPad and can crop the title/buttons.
        #[cfg(target_os = "ios")]
        let attributes = attributes
            .with_visible(!self.options.as_ref().is_some_and(|o| o.offscreen));
        #[cfg(not(target_os = "ios"))]
        let attributes = attributes
            .with_inner_size(winit::dpi::LogicalSize::new(size[0], size[1]))
            .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None)))
            .with_visible(!self.options.as_ref().is_some_and(|o| o.offscreen));
        let window = match event_loop.create_window(attributes) {""",
)
change(
    loading,
    """        horizon_client::window::settle_created_fullscreen(&window, fullscreen);""",
    """        #[cfg(not(target_os = "ios"))]
        horizon_client::window::settle_created_fullscreen(&window, fullscreen);
        #[cfg(target_os = "ios")]
        nova_diag(&format!(
            "ios.window.bounds; physical={}x{}; scale={}",
            window.inner_size().width,
            window.inner_size().height,
            window.scale_factor()
        ));""",
)

game = "crates/horizon-client/src/game/window_handler.rs"
change(
    game,
    """        let window = match event_loop.create_window(
            attributes
                .with_inner_size(winit::dpi::LogicalSize::new(size[0], size[1]))
                .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None)))
                .with_visible(!self.options.offscreen)
                .with_active(!agent_started),
        ) {""",
    """        #[cfg(target_os = "ios")]
        let attributes = attributes
            .with_visible(!self.options.offscreen)
            .with_active(!agent_started);
        #[cfg(not(target_os = "ios"))]
        let attributes = attributes
            .with_inner_size(winit::dpi::LogicalSize::new(size[0], size[1]))
            .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None)))
            .with_visible(!self.options.offscreen)
            .with_active(!agent_started);
        let window = match event_loop.create_window(attributes) {""",
)
change(
    game,
    """        horizon_client::window::settle_created_fullscreen(&window, fullscreen);""",
    """        #[cfg(not(target_os = "ios"))]
        horizon_client::window::settle_created_fullscreen(&window, fullscreen);""",
)

# deep_stack_callbacks.py has already lifted the original App handler bodies
# into an inherent impl App, keeping same-thread UIKit/Metal callbacks intact.
ios_touch = """impl App {
    /// Primary-finger touch -> existing cursor/mouse input. All events travel
    /// through Horizon's original menu/game handlers. Multi-touch contacts
    /// never steal the active finger or leave a held mouse button behind.
    #[cfg(target_os = "ios")]
    fn ios_touch_mouse(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: WindowId,
        touch: &winit::event::Touch,
    ) {
        use std::sync::atomic::{AtomicU64, Ordering};
        use winit::event::TouchPhase;
        const EMPTY: u64 = u64::MAX;
        static ACTIVE: AtomicU64 = AtomicU64::new(EMPTY);

        let accepted = match touch.phase {
            TouchPhase::Started => ACTIVE
                .compare_exchange(EMPTY, touch.id, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok(),
            TouchPhase::Moved => ACTIVE.load(Ordering::SeqCst) == touch.id,
            TouchPhase::Ended | TouchPhase::Cancelled => ACTIVE
                .compare_exchange(touch.id, EMPTY, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok(),
        };
        if !accepted {
            return;
        }
        self.window_event_here(
            event_loop,
            id,
            WindowEvent::CursorMoved {
                device_id: DeviceId::dummy(),
                position: touch.location,
            },
        );
        let state = match touch.phase {
            TouchPhase::Started => Some(ElementState::Pressed),
            TouchPhase::Ended | TouchPhase::Cancelled => Some(ElementState::Released),
            TouchPhase::Moved => None,
        };
        if let Some(state) = state {
            self.window_event_here(
                event_loop,
                id,
                WindowEvent::MouseInput {
                    device_id: DeviceId::dummy(),
                    state,
                    button: MouseButton::Left,
                },
            );
        }
    }
"""
change(
    loading,
    "impl App {\n",
    ios_touch,
)
change(
    loading,
    """    fn window_event_here(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match self {""",
    """    fn window_event_here(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        #[cfg(target_os = "ios")]
        if let WindowEvent::Touch(touch) = &event {
            self.ios_touch_mouse(event_loop, id, touch);
            return;
        }
        match self {""",
)
print("Nova iOS: native screen bounds, hidden system bars, landscape and touch click/drag")
