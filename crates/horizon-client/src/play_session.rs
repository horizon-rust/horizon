//! A play session as the hooks above the engine hear it (`horizon_hooks`): the title screen's
//! first frame, a world opening and its first frame, the frames played, and leaving the world.
//! It also marks those moments in the startup trace (`spans::startup`). Opt-in telemetry's
//! performance summary is built from these hooks (docs/telemetry.md).
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use horizon_hooks::{Frame, Milestone, WorldLeft, engine};

use crate::spans::{self, startup};

/// One window's session: moved from the startup menu to the game with the window.
#[derive(Default)]
pub struct Session {
    /// When the world being opened started to, until its first frame.
    opening: Option<Instant>,
}

impl Session {
    /// The title screen drew a frame: the first time, how long the start took.
    pub fn title_shown(&mut self) {
        static SHOWN: AtomicBool = AtomicBool::new(false);
        if !SHOWN.swap(true, Ordering::Relaxed) {
            startup::mark("title.shown");
            // The frame itself: the menu bundle can be ready seconds before the renderer that
            // draws it (pipelines compiling on a slow driver).
            engine::startup(Milestone::TitleShown, startup::since_start());
        }
    }

    /// A world starts loading.
    pub fn world_opening(&mut self) {
        if self.opening.is_none() {
            startup::arm("world.opening");
        }
        self.opening.get_or_insert_with(Instant::now);
    }

    /// An entry queued during startup was cancelled before opening a world.
    pub fn cancel_world_opening(&mut self) {
        self.opening = None;
    }

    /// A frame drawn while playing (not in menus or loading), with the spans the render thread
    /// recorded in it ([`spans::take_raw`]): the step it spent most of its time in is
    /// [`spans::culprit`].
    pub fn frame(&mut self, ms: f32, frame_spans: &[spans::Span]) {
        if let Some(opening) = self.opening.take() {
            startup::mark("world.first_frame");
            engine::world_shown(opening.elapsed());
        }
        engine::frame(&Frame::new(ms, &|| spans::culprit(frame_spans)));
    }

    /// The player left the world: the session ends.
    pub fn finish(&mut self, left: &WorldLeft<'_>) {
        engine::world_left(left);
    }
}

/// The resources the menus need are loaded.
pub fn core_ready() {
    engine::startup(Milestone::CoreReady, startup::since_start());
}

/// What a world needs is loaded too.
pub fn rest_ready() {
    engine::startup(Milestone::RestReady, startup::since_start());
}
