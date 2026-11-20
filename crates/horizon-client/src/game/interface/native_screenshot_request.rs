//! `ClientHost::request_screenshot`: a client mod takes a screenshot through F2's
//! own path (`Game::take_screenshot`), so the `screenshot` hook, the file name and
//! the chat line are F2's. Unlike F2's, its files never replace one
//! (`PendingScreenshot::no_replace`), and all mods share one budget per session.
use super::Game;
use mod_api::ScreenshotError;

/// Uncompressed bytes all mods' screenshots may write per game session: about 500
/// at 1920 by 1080, far past what a screenshot mod takes in play, but a bound on a
/// mod that captures on every tick.
pub(super) const MOD_SCREENSHOT_BUDGET: u64 = 4 << 30;

impl Game<'_> {
    pub(in crate::game) fn request_screenshot(&mut self) -> Result<(), ScreenshotError> {
        // A server thread's `Game` has no window to grab, as `narrate` says for
        // the narrator.
        let available = self.options.execution_role.is_client() && self.renderer.is_some();
        // Everything that makes a mod's request wait lives in the renderer or ends on
        // its own: a capture queued or read back (dropped with the renderer when a
        // reload replaces it), the hook's run, a write in progress, a sequence. F2's
        // replace branch would otherwise drop a queued capture (another F2, a world
        // icon, an agent capture).
        let busy = self.ui.screenshot_hook
            || !self.ui.screenshot_writes.is_empty()
            || self.screenshot_sequence.is_some()
            || self
                .renderer
                .as_ref()
                .is_some_and(|r| r.screenshot_in_flight());
        if let Some(refusal) = screenshot_refusal(available, busy, self.ui.mod_screenshot_bytes) {
            return Err(refusal);
        }
        // Past the checks above, `take_screenshot` only declines when it cannot
        // create the screenshots folder.
        let path = self.take_screenshot().ok_or(ScreenshotError::Folder)?;
        self.ui.mod_screenshot = Some(path);
        Ok(())
    }
}

/// Why a mod's screenshot is refused before anything is queued, if it is: no
/// window, one still in progress, or the session's budget used up.
pub(super) fn screenshot_refusal(
    available: bool,
    busy: bool,
    written: u64,
) -> Option<ScreenshotError> {
    if !available {
        Some(ScreenshotError::Unavailable)
    } else if busy {
        Some(ScreenshotError::Busy)
    } else if written >= MOD_SCREENSHOT_BUDGET {
        Some(ScreenshotError::BudgetExhausted)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{MOD_SCREENSHOT_BUDGET, screenshot_refusal};
    use mod_api::ScreenshotError;

    #[test]
    fn a_window_with_nothing_in_progress_and_budget_left_is_accepted() {
        assert_eq!(screenshot_refusal(true, false, 0), None);
        assert_eq!(
            screenshot_refusal(true, false, MOD_SCREENSHOT_BUDGET - 1),
            None
        );
        assert_eq!(
            screenshot_refusal(false, false, 0),
            Some(ScreenshotError::Unavailable)
        );
        assert_eq!(
            screenshot_refusal(true, true, 0),
            Some(ScreenshotError::Busy)
        );
        assert_eq!(
            screenshot_refusal(true, false, MOD_SCREENSHOT_BUDGET),
            Some(ScreenshotError::BudgetExhausted)
        );
    }
}
