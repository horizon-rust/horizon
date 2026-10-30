//! Nintendo Switch homebrew (`target_os = "horizon"`, libnx). One screen, one window (libnx's
//! default `NWindow`), the touch screen as the pointer, and the applet loop for HOME and docking.
//! The controllers are not here: the game reads them through `horizon-switch`.
#![cfg(horizon_platform)]

use std::fmt::{self, Display, Formatter};

use smol_str::SmolStr;

use crate::dpi::{PhysicalPosition, PhysicalSize};
use crate::keyboard::Key;

pub(crate) use self::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy, OwnedDisplayHandle};
mod event_loop;

pub use self::window::Window;
mod window;

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PlatformSpecificEventLoopAttributes {}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct WindowId;

impl WindowId {
    pub const fn dummy() -> Self {
        WindowId
    }
}

impl From<WindowId> for u64 {
    fn from(_: WindowId) -> Self {
        1
    }
}

impl From<u64> for WindowId {
    fn from(_: u64) -> Self {
        WindowId
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeviceId;

impl DeviceId {
    pub const fn dummy() -> Self {
        DeviceId
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PlatformSpecificWindowAttributes;

#[derive(Clone, Debug)]
pub struct OsError(&'static str);

impl Display for OsError {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), fmt::Error> {
        fmt.write_str(self.0)
    }
}

pub(crate) use crate::cursor::{
    NoCustomCursor as PlatformCustomCursor, NoCustomCursor as PlatformCustomCursorSource,
};
pub(crate) use crate::icon::NoIcon as PlatformIcon;

/// The screen: 1280x720 in handheld mode, 1920x1080 docked.
pub(crate) fn screen_size() -> PhysicalSize<u32> {
    if horizon_switch::Libnx::docked() {
        PhysicalSize::new(1920, 1080)
    } else {
        PhysicalSize::new(1280, 720)
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MonitorHandle;

impl MonitorHandle {
    pub fn name(&self) -> Option<String> {
        Some("Nintendo Switch".to_owned())
    }

    pub fn size(&self) -> PhysicalSize<u32> {
        screen_size()
    }

    pub fn position(&self) -> PhysicalPosition<i32> {
        (0, 0).into()
    }

    pub fn scale_factor(&self) -> f64 {
        1.0
    }

    pub fn refresh_rate_millihertz(&self) -> Option<u32> {
        Some(60000)
    }

    pub fn video_modes(&self) -> impl Iterator<Item = VideoModeHandle> {
        std::iter::once(VideoModeHandle { size: self.size().into(), monitor: self.clone() })
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct VideoModeHandle {
    size: (u32, u32),
    monitor: MonitorHandle,
}

impl VideoModeHandle {
    pub fn size(&self) -> PhysicalSize<u32> {
        self.size.into()
    }

    pub fn bit_depth(&self) -> u16 {
        32
    }

    pub fn refresh_rate_millihertz(&self) -> u32 {
        60000
    }

    pub fn monitor(&self) -> MonitorHandle {
        self.monitor.clone()
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct KeyEventExtra {
    pub key_without_modifiers: Key,
    pub text_with_all_modifiers: Option<SmolStr>,
}
