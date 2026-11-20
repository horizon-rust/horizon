/// The in-memory F2 frame (`ScreenshotEvent.getImage`), before either file is written.
///
/// RGBA8, top to bottom: `rgba.len()` is `width * height * 4`. Change the pixels, or
/// replace the whole image, and the normal PNG is what remains. A length other than
/// `width * height * 4` writes nothing and reports screenshot failure. The HDR copy
/// is not this image.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ScreenshotImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA8.
    pub rgba: Vec<u8>,
}

impl ScreenshotImage {
    /// An RGBA8 frame of `width` by `height`.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Self {
        Self {
            width,
            height,
            rgba,
        }
    }
}

/// Why `ClientHost::request_screenshot` did not queue a screenshot. Nothing was
/// queued, written or posted in chat in any case; each cause has its own name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum ScreenshotError {
    /// A size was asked for: only `None`, the frame F2 saves, is supported.
    UnsupportedSize,
    /// A screenshot is already in progress: an F2 or mod screenshot not yet
    /// reported in chat (the `screenshot` hook's own run included), another
    /// capture queued for the next frame, or a screenshot sequence running.
    Busy,
    /// The player's `screenshots` folder could not be created.
    Folder,
    /// No window renders here: a server's thread, a client with no renderer
    /// yet, or no game backend.
    Unavailable,
}

impl std::fmt::Display for ScreenshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedSize => f.write_str("only the size F2 saves is supported"),
            Self::Busy => f.write_str("a screenshot is already in progress"),
            Self::Folder => f.write_str("the screenshots folder could not be created"),
            Self::Unavailable => f.write_str("no window renders here"),
        }
    }
}

impl std::error::Error for ScreenshotError {}

impl crate::NoBackend for ScreenshotError {
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
