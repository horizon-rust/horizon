//! What a `Game` is to splitscreen: alone, P1 of a split, or another player's game. One value
//! the engine asks, in place of the separate flags it used to keep (spec #14206, step 1).
use std::sync::Arc;

use horizon_client::auth::AuthSession;

/// A `Game`'s place in a split. Only the host and the guests are in a split.
#[derive(Clone, Default)]
pub(super) enum Role {
    /// No splitscreen: the game plays as it always did, with its own pacing and window.
    #[default]
    Alone,
    /// P1 while others play: the split window paces every player's frames. `window`: the
    /// split's window is a visible one, which accepts native input.
    Host { window: bool },
    /// Another player. Its windows are the split's (P1 alone decides fullscreen; this game
    /// neither asks for it nor adopts it into its options), and it has nothing to save.
    /// `window`: as for the host. `auth`: this player's own Microsoft account session (one
    /// of the launcher's other accounts), in place of the process's (P1's).
    Guest {
        window: bool,
        auth: Option<Arc<AuthSession>>,
    },
}

impl Role {
    pub(super) fn is_guest(&self) -> bool {
        matches!(self, Self::Guest { .. })
    }
    /// The split window paces this game's frames, not the game itself.
    pub(super) fn paced(&self) -> bool {
        !matches!(self, Self::Alone)
    }
    /// The game is drawn into a visible split window, which accepts native input.
    pub(super) fn in_window(&self) -> bool {
        matches!(
            self,
            Self::Host { window: true } | Self::Guest { window: true, .. }
        )
    }
    /// Gives a guest its own account; other roles have none to take.
    pub(super) fn set_guest_auth(&mut self, account: Option<Arc<AuthSession>>) {
        if let Self::Guest { auth, .. } = self {
            *auth = account;
        }
    }
}
