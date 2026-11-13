//! When a large translucent page may start to shrink (`Renderer::shrink_translucent`).

use std::time::Instant;

/// A large translucent page starts to shrink (`Renderer::shrink_translucent`)
/// this long after terrain last arrived, and `SHRINK_RETRY` after a try found
/// no room.
pub(super) const SHRINK_SETTLE: std::time::Duration = std::time::Duration::from_secs(1);
pub(super) const SHRINK_RETRY: std::time::Duration = std::time::Duration::from_secs(5);
/// Not before then may a large translucent page start to shrink.
#[derive(Default)]
pub(super) struct ShrinkGate(Option<Instant>);
impl ShrinkGate {
    /// Not before `wait` from `now`, nor before an earlier wait is over.
    pub(super) fn wait(&mut self, now: Instant, wait: std::time::Duration) {
        self.0 = self.0.max(Some(now + wait));
    }
    /// Whether one may start now: the waits are over, and `compact_arena` copied
    /// nothing this frame (it `left` the whole budget).
    pub(super) fn may_start(&self, now: Instant, left: u64) -> bool {
        left == super::COMPACT_BUDGET && self.0.is_none_or(|at| now >= at)
    }
}
