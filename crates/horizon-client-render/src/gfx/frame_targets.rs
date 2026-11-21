//! Targets a frame makes on demand, made so that running out of video memory is not fatal
//! (#13624). Once terrain has filled video memory, a texture made outside an error scope that
//! finds no room reaches wgpu's default error handler, which panics. These are made under
//! out-of-memory and validation scopes instead (a texture that failed is invalid, and so are its
//! views and bind groups), and what failed is dropped, never kept: an effect the frame can do
//! without is left out, and a frame without a target it needs is skipped and tried again.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// What had no room, for its warning: each is logged once.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Kind {
    MenuBlur,
    HdrFrame,
    Depth,
    Readback,
    Mirror,
    Outline,
    Transparency,
    Visibility,
    FrameGeneration,
}

impl Kind {
    fn consequence(self) -> &'static str {
        match self {
            Kind::MenuBlur => "menus draw without the background blur",
            Kind::HdrFrame => "no HDR frame; frames are skipped until there is room for it",
            Kind::Depth => "no window depth; frames are skipped until there is room for it",
            Kind::Readback => "no frame target; frames are skipped until there is room for it",
            Kind::Mirror => "water draws without its reflection",
            Kind::Outline => "glowing entities draw without their outline",
            Kind::Transparency => "Improved Transparency is off until the window's size changes",
            Kind::Visibility => "solid terrain draws without the visibility buffer",
            Kind::FrameGeneration => "frame generation is off until the window's size changes",
        }
    }
}

/// The kinds warned of, one bit each.
static WARNED: AtomicU32 = AtomicU32::new(0);
/// A window frame raised an error none of its own fallbacks caught (warned of once).
static FRAME_WARNED: AtomicBool = AtomicBool::new(false);

thread_local! {
    /// A scaled screenshot's frame is being drawn (`in_screenshot`).
    static SCREENSHOT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
thread_local! {
    /// Tests: the next this many `made` report no room, as a device out of video memory would.
    pub(crate) static NO_ROOM: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Draws a scaled screenshot's frame (`draw`), in which `made` makes what it is given without
/// scopes of its own: the frame's own scopes see a failure and take the screenshot again at
/// half the size, which a target left out would not.
pub(crate) fn in_screenshot<T>(draw: impl FnOnce() -> T) -> T {
    let was = SCREENSHOT.replace(true);
    let drawn = draw();
    SCREENSHOT.set(was);
    drawn
}

/// What `make` makes, or `None` where it raised an out-of-memory or validation error (warned of
/// once per `kind`); what it made is dropped then. Native error scopes resolve at once, so the
/// wait for them costs nothing.
pub(crate) fn made<T>(device: &wgpu::Device, kind: Kind, make: impl FnOnce() -> T) -> Option<T> {
    if SCREENSHOT.get() {
        return Some(make());
    }
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let value = make();
    let error = pollster::block_on(memory.pop()).or(pollster::block_on(validation.pop()));
    let error = error.map(|e| e.to_string());
    #[cfg(test)]
    let error = error.or_else(|| {
        let left = NO_ROOM.get().checked_sub(1)?;
        NO_ROOM.set(left);
        Some("no room (test)".to_owned())
    });
    let Some(error) = error else {
        return Some(value);
    };
    let bit = 1 << kind as u32;
    if WARNED.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
        log::warn!(
            "Horizon renderer: video memory is full; {} ({error})",
            kind.consequence()
        );
    }
    None
}

/// Warns, once, of an error a window frame raised that no fallback of its own caught: the
/// frame scope's backstop (`Renderer::render`), in place of wgpu's handler, which panics.
pub(crate) fn frame_failed(error: &wgpu::Error) {
    if !FRAME_WARNED.swap(true, Ordering::Relaxed) {
        log::warn!("Horizon renderer: a frame raised an error and may show wrong ({error})");
    }
}
