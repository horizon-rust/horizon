use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::cursor::Cursor;
use crate::dpi::{PhysicalPosition, PhysicalSize, Position, Size};
use crate::error;
use crate::platform_impl::Fullscreen;
use crate::window::{self, ImePurpose};

use super::{ActiveEventLoop, MonitorHandle, WindowId};

#[cfg(feature = "rwh_06")]
unsafe extern "C" {
    /// libnx: the window for the application's default display layer.
    fn nwindowGetDefault() -> *mut c_void;
}

/// The Switch's one window: libnx's default `NWindow`, always the whole screen.
pub struct Window {
    redraw: Arc<AtomicBool>,
}

impl Window {
    pub(crate) fn new(
        el: &ActiveEventLoop,
        _attrs: window::WindowAttributes,
    ) -> Result<Self, error::OsError> {
        el.window_created.set(true);
        Ok(Self { redraw: el.redraw.clone() })
    }

    pub(crate) fn maybe_queue_on_main(&self, f: impl FnOnce(&Self) + Send + 'static) {
        f(self)
    }

    pub(crate) fn maybe_wait_on_main<R: Send>(&self, f: impl FnOnce(&Self) -> R + Send) -> R {
        f(self)
    }

    #[inline]
    pub fn id(&self) -> WindowId {
        WindowId
    }

    #[inline]
    pub fn primary_monitor(&self) -> Option<MonitorHandle> {
        Some(MonitorHandle)
    }

    #[inline]
    pub fn available_monitors(&self) -> VecDeque<MonitorHandle> {
        VecDeque::from([MonitorHandle])
    }

    #[inline]
    pub fn current_monitor(&self) -> Option<MonitorHandle> {
        Some(MonitorHandle)
    }

    #[inline]
    pub fn scale_factor(&self) -> f64 {
        1.0
    }

    #[inline]
    pub fn request_redraw(&self) {
        self.redraw.store(true, Ordering::Relaxed);
    }

    #[inline]
    pub fn pre_present_notify(&self) {}

    #[inline]
    pub fn reset_dead_keys(&self) {}

    #[inline]
    pub fn inner_position(&self) -> Result<PhysicalPosition<i32>, error::NotSupportedError> {
        Ok((0, 0).into())
    }

    #[inline]
    pub fn outer_position(&self) -> Result<PhysicalPosition<i32>, error::NotSupportedError> {
        Ok((0, 0).into())
    }

    #[inline]
    pub fn set_outer_position(&self, _position: Position) {}

    #[inline]
    pub fn inner_size(&self) -> PhysicalSize<u32> {
        super::screen_size()
    }

    #[inline]
    pub fn request_inner_size(&self, _size: Size) -> Option<PhysicalSize<u32>> {
        Some(self.inner_size())
    }

    #[inline]
    pub fn outer_size(&self) -> PhysicalSize<u32> {
        self.inner_size()
    }

    #[inline]
    pub fn set_min_inner_size(&self, _: Option<Size>) {}

    #[inline]
    pub fn set_max_inner_size(&self, _: Option<Size>) {}

    #[inline]
    pub fn title(&self) -> String {
        String::new()
    }

    #[inline]
    pub fn set_title(&self, _title: &str) {}

    #[inline]
    pub fn set_transparent(&self, _transparent: bool) {}

    #[inline]
    pub fn set_blur(&self, _blur: bool) {}

    #[inline]
    pub fn set_visible(&self, _visible: bool) {}

    #[inline]
    pub fn is_visible(&self) -> Option<bool> {
        Some(true)
    }

    #[inline]
    pub fn resize_increments(&self) -> Option<PhysicalSize<u32>> {
        None
    }

    #[inline]
    pub fn set_resize_increments(&self, _increments: Option<Size>) {}

    #[inline]
    pub fn set_resizable(&self, _resizeable: bool) {}

    #[inline]
    pub fn is_resizable(&self) -> bool {
        false
    }

    #[inline]
    pub fn set_minimized(&self, _minimized: bool) {}

    #[inline]
    pub fn is_minimized(&self) -> Option<bool> {
        Some(false)
    }

    #[inline]
    pub fn set_maximized(&self, _maximized: bool) {}

    #[inline]
    pub fn is_maximized(&self) -> bool {
        true
    }

    #[inline]
    pub(crate) fn set_fullscreen(&self, _monitor: Option<Fullscreen>) {}

    #[inline]
    pub(crate) fn fullscreen(&self) -> Option<Fullscreen> {
        Some(Fullscreen::Borderless(None))
    }

    #[inline]
    pub fn set_decorations(&self, _decorations: bool) {}

    #[inline]
    pub fn is_decorated(&self) -> bool {
        false
    }

    #[inline]
    pub fn set_window_level(&self, _level: window::WindowLevel) {}

    #[inline]
    pub fn set_window_icon(&self, _window_icon: Option<crate::icon::Icon>) {}

    #[inline]
    pub fn set_ime_cursor_area(&self, _position: Position, _size: Size) {}

    #[inline]
    pub fn set_ime_allowed(&self, _allowed: bool) {}

    #[inline]
    pub fn set_ime_purpose(&self, _purpose: ImePurpose) {}

    #[inline]
    pub fn focus_window(&self) {}

    #[inline]
    pub fn request_user_attention(&self, _request_type: Option<window::UserAttentionType>) {}

    #[inline]
    pub fn set_cursor(&self, _: Cursor) {}

    #[inline]
    pub fn set_cursor_position(&self, _: Position) -> Result<(), error::ExternalError> {
        Err(error::ExternalError::NotSupported(error::NotSupportedError::new()))
    }

    /// There is no OS cursor to grab: the game's own pointer logic works on the touch events.
    #[inline]
    pub fn set_cursor_grab(
        &self,
        _mode: window::CursorGrabMode,
    ) -> Result<(), error::ExternalError> {
        Ok(())
    }

    #[inline]
    pub fn set_cursor_visible(&self, _visible: bool) {}

    #[inline]
    pub fn drag_window(&self) -> Result<(), error::ExternalError> {
        Err(error::ExternalError::NotSupported(error::NotSupportedError::new()))
    }

    #[inline]
    pub fn drag_resize_window(
        &self,
        _direction: window::ResizeDirection,
    ) -> Result<(), error::ExternalError> {
        Err(error::ExternalError::NotSupported(error::NotSupportedError::new()))
    }

    #[inline]
    pub fn show_window_menu(&self, _position: Position) {}

    #[inline]
    pub fn set_cursor_hittest(&self, _hittest: bool) -> Result<(), error::ExternalError> {
        Err(error::ExternalError::NotSupported(error::NotSupportedError::new()))
    }

    #[cfg(feature = "rwh_06")]
    #[inline]
    pub fn raw_window_handle_rwh_06(&self) -> Result<rwh_06::RawWindowHandle, rwh_06::HandleError> {
        // SAFETY: plain libnx call; the default NWindow lives as long as the process.
        let n_window = std::ptr::NonNull::new(unsafe { nwindowGetDefault() })
            .ok_or(rwh_06::HandleError::Unavailable)?;
        Ok(rwh_06::RawWindowHandle::Horizon(rwh_06::HorizonWindowHandle::new(n_window)))
    }

    #[cfg(feature = "rwh_06")]
    #[inline]
    pub fn raw_display_handle_rwh_06(
        &self,
    ) -> Result<rwh_06::RawDisplayHandle, rwh_06::HandleError> {
        Ok(rwh_06::RawDisplayHandle::Horizon(rwh_06::HorizonDisplayHandle::new()))
    }

    #[inline]
    pub fn set_enabled_buttons(&self, _buttons: window::WindowButtons) {}

    #[inline]
    pub fn enabled_buttons(&self) -> window::WindowButtons {
        window::WindowButtons::empty()
    }

    #[inline]
    pub fn theme(&self) -> Option<window::Theme> {
        None
    }

    #[inline]
    pub fn has_focus(&self) -> bool {
        horizon_switch::Libnx::in_focus()
    }

    #[inline]
    pub fn set_theme(&self, _theme: Option<window::Theme>) {}

    pub fn set_content_protected(&self, _protected: bool) {}
}
