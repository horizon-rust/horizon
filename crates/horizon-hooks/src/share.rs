//! Sharing the open world past the LAN (Play with Friends' host side): what the engine lets a
//! crate do with the world a window plays. The engine implements [`ShareView`] and hands it to
//! [`Hooks::share`](crate::Hooks::share) once a client tick; [`Hooks::lan_closed`](crate::Hooks::lan_closed)
//! says the world stopped being open to LAN.
//!
//! Only the window's own player calls them (a splitscreen guest can't share P1's world, and
//! its leaving doesn't close it), never the integrated server's thread or a dedicated server.

/// How [`ShareView::open_lan`] opens the world, as Open to LAN's screen does. The default is
/// what `ShareToLanScreen` offers: no commands for guests, the world's game mode forced on them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LanOptions {
    /// Guests may run commands (`ShareToLanScreen`'s "Allow Cheats").
    pub guest_commands: bool,
    /// Guests play in the world's game mode.
    pub force_game_mode: bool,
}

impl Default for LanOptions {
    fn default() -> Self {
        Self {
            guest_commands: false,
            force_game_mode: true,
        }
    }
}

/// The window's world, as a crate sharing it sees it. Nothing here allocates per call unless a
/// method says so.
pub trait ShareView {
    /// A singleplayer world runs on this window's server thread, so it can be shared. False on
    /// the title screen and on another server.
    fn can_share(&self) -> bool;
    /// The port the world is open to LAN on, if it is.
    fn lan_port(&self) -> Option<u16>;
    /// Opens the world to LAN on a free port, as Open to LAN does; the port, or why not.
    fn open_lan(&mut self, options: LanOptions) -> Result<u16, String>;
    /// Closes the world to LAN, saying so in chat as Open to LAN's stop does.
    fn close_lan(&mut self);
    /// The names of the other players in the world (not this one's). For a screen or a command,
    /// not every tick.
    fn players(&self, each: &mut dyn FnMut(&str));
    /// A line in this player's chat, as the game's own system messages; `text` is already in
    /// the player's language ([`ShareView::tr`]).
    fn chat(&mut self, text: &str);
    /// Puts `text` on the system clipboard.
    fn copy(&mut self, text: &str);
    /// Opens `url` in the system's browser.
    fn open_url(&mut self, url: &str);
    /// The translation of `key` in the player's language, if the game has one.
    fn tr(&self, key: &str) -> Option<String>;
}
