//! What the custom entity models' animations ([`super::cem`]) read besides the entity: the
//! frame every entity of it shares, which the game sets once per frame, and whether an entity
//! is drawn as a screen's preview.

use std::{
    cell::Cell,
    sync::{Mutex, MutexGuard, PoisonError},
};

/// What every entity of a frame shares, set once per frame by the game.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// The world's game time and day time in ticks, with the partial tick.
    pub time: f64,
    pub day_time: f64,
    pub frame_time: f32,
    pub frame_counter: u32,
    /// Overworld 0, nether -1, end 1.
    pub dimension: f32,
    pub player: [f64; 3],
    pub player_rot: [f32; 2],
    pub paused: bool,
    pub first_person: bool,
    pub third_person_reversed: bool,
}

impl Frame {
    /// Before the first frame: in the overworld at the origin, seen in first person.
    pub const DEFAULT: Self = Self {
        time: 0.0,
        day_time: 0.0,
        frame_time: 0.0,
        frame_counter: 0,
        dimension: 0.0,
        player: [0.0; 3],
        player_rot: [0.0; 2],
        paused: false,
        first_person: true,
        third_person_reversed: false,
    };
}

impl Default for Frame {
    fn default() -> Self {
        Self::DEFAULT
    }
}

static FRAME: Mutex<Frame> = Mutex::new(Frame::DEFAULT);

/// `m`'s value, even if a thread panicked holding it.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The frame the next animations see. Its `frame_time` and `frame_counter` come from the
/// calls themselves: the time since the previous one, and how many there were.
pub fn set_frame(mut frame: Frame) {
    static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    let now = std::time::Instant::now();
    let mut last = lock(&LAST);
    frame.frame_time = last.map_or(0.0, |t| (now - t).as_secs_f32());
    *last = Some(now);
    let mut current = lock(&FRAME);
    frame.frame_counter = current.frame_counter.wrapping_add(1);
    *current = frame;
}

/// The frame the animations see ([`set_frame`]).
pub fn frame() -> Frame {
    *lock(&FRAME)
}

thread_local!(static IN_GUI: Cell<bool> = const { Cell::new(false) });

/// Runs `draw` as a screen's preview of an entity (the inventory's player, a mount's box):
/// its animations see `is_in_gui = 1` and store no variables, unless the world didn't draw
/// the entity this frame or the last. Screens draw before the world in a frame, so a
/// preview runs under the previous frame's counter and time. `set_frame` must not be
/// called inside it.
pub fn in_gui<R>(draw: impl FnOnce() -> R) -> R {
    /// Puts the previous value back, even if `draw` panics.
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            IN_GUI.set(self.0);
        }
    }
    let _restore = Restore(IN_GUI.replace(true));
    draw()
}

/// Whether this thread is drawing a screen's preview ([`in_gui`]).
pub fn is_in_gui() -> bool {
    IN_GUI.get()
}
