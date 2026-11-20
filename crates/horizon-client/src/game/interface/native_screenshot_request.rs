//! `ClientHost::request_screenshot`: a client mod takes a screenshot through F2's
//! own path (`Game::take_screenshot`), so the `screenshot` hook, the file name and
//! the chat line are F2's.
use super::Game;
use mod_api::ScreenshotError;

impl Game<'_> {
    pub(in crate::game) fn request_screenshot(
        &mut self,
        size: Option<[u32; 2]>,
    ) -> Result<(), ScreenshotError> {
        // A server thread's `Game` has no window to grab, as `narrate` says for
        // the narrator.
        let available = self.options.execution_role.is_client() && self.renderer.is_some();
        // F2's own replace branch would drop a capture queued for the next frame
        // (another F2, a world icon, an agent capture), and a call from inside the
        // `screenshot` hook finds its own screenshot still listed.
        let busy = !self.ui.screenshot_messages.is_empty()
            || self.screenshot_sequence.is_some()
            || self
                .renderer
                .as_ref()
                .is_some_and(|r| r.screenshot.is_some());
        if let Some(refusal) = screenshot_refusal(size, available, busy) {
            return Err(refusal);
        }
        // Past the checks above, `take_screenshot` only declines when it cannot
        // create the screenshots folder.
        match self.take_screenshot() {
            Some(_) => Ok(()),
            None => Err(ScreenshotError::Folder),
        }
    }
}

/// Why a mod's screenshot is refused before anything is queued, if it is: a size
/// (only F2's is supported), no window, or one still in progress.
pub(super) fn screenshot_refusal(
    size: Option<[u32; 2]>,
    available: bool,
    busy: bool,
) -> Option<ScreenshotError> {
    if size.is_some() {
        Some(ScreenshotError::UnsupportedSize)
    } else if !available {
        Some(ScreenshotError::Unavailable)
    } else if busy {
        Some(ScreenshotError::Busy)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::screenshot_refusal;
    use mod_api::ScreenshotError;

    #[test]
    fn only_f2s_size_with_a_window_and_nothing_in_progress_is_accepted() {
        assert_eq!(screenshot_refusal(None, true, false), None);
        assert_eq!(
            screenshot_refusal(Some([1920, 1080]), true, false),
            Some(ScreenshotError::UnsupportedSize)
        );
        assert_eq!(
            screenshot_refusal(None, false, false),
            Some(ScreenshotError::Unavailable)
        );
        assert_eq!(
            screenshot_refusal(None, true, true),
            Some(ScreenshotError::Busy)
        );
        // A size is refused first: it never works, whatever the window does.
        assert_eq!(
            screenshot_refusal(Some([64, 64]), false, true),
            Some(ScreenshotError::UnsupportedSize)
        );
    }
}
