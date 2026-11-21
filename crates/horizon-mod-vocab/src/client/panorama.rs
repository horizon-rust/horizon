//! A 360-degree capture of the world, asked for by a client mod (`ClientHost::panorama`).

/// How a panorama is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PanoramaFormat {
    /// Six PNGs, one per cube face: `<name>_south.png`, `_west`, `_north`, `_east`, `_up`, `_down`.
    Cube,
    /// One equirectangular PNG: the player's heading in the middle column, straight up on the top
    /// row, four faces wide and two tall (at most 8192 by 4096).
    Equirect,
}

/// What a mod asks for with `ClientHost::panorama`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct PanoramaRequest {
    /// The file name, relative to `screenshots/panorama` in the player's game folder
    /// (`night/view.png`); folders are created. A name with a backslash, a colon, a `..` part or a
    /// symbolic link on its way is refused, as the screenshot writer refuses it.
    pub path: String,
    /// Each cube face's side in pixels, 16 to 4096.
    pub size: u32,
    /// One equirectangular picture or six cube faces.
    pub format: PanoramaFormat,
}

impl PanoramaRequest {
    /// An equirectangular capture of `path` with faces of `size` pixels.
    pub fn equirect(path: impl Into<String>, size: u32) -> Self {
        Self {
            path: path.into(),
            size,
            format: PanoramaFormat::Equirect,
        }
    }
}

/// Why `ClientHost::panorama` refused. Nothing was started.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PanoramaRefusal {
    /// Agent Control is off: the capture is part of it, and runs only while it is on.
    AgentControlOff,
    /// There is no renderer (`--headless`): the answer `screenshot` gives there.
    NoRenderer,
    /// No world is shown (the title screen).
    NoWorld,
    /// A capture is already running (a mod's or an agent's); ask again when it is done.
    Busy,
    /// The size is outside 16 to 4096, or beyond what the device draws; the text says which.
    Size(String),
    /// The file name is not a plain relative path.
    Path(String),
}

/// How a capture asked for with `ClientHost::panorama` stands.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PanoramaStatus {
    /// Still drawing its six faces or writing the files.
    Pending,
    /// Written: the files' paths.
    Done(Vec<String>),
    /// Stopped, with the reason.
    Failed(String),
    /// Not a ticket this game gave, or one of the last eight finished ones' older kin.
    Unknown,
}
