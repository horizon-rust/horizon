//! The window's world as `horizon_hooks::share` shows it to the crates that share it past the
//! LAN (Play with Friends), and where the engine calls `Hooks::share` and `Hooks::lan_closed`.
use super::{Game, lan::LanSettings};
use horizon_client::text::Text;
use horizon_hooks::share::{LanOptions, ShareView};

/// The window's own player: not the integrated server's thread, not a splitscreen guest (it
/// plays P1's world and can't share it; its leaving doesn't close it).
fn shares(game: &Game<'_>) -> bool {
    !game.serving() && !game.options.split_guest
}

impl Game<'_> {
    /// `Hooks::share`, once a client tick (`Minecraft.runTick`, so also while paused).
    pub(super) fn share_tick(&mut self) {
        if shares(self) {
            horizon_hooks::engine::share(&mut World(self));
        }
    }

    /// `Hooks::lan_closed`: the world stopped being open to LAN, or the player left it.
    pub(super) fn lan_closed_hooks(&self) {
        if shares(self) {
            horizon_hooks::engine::lan_closed();
        }
    }

    /// `Hooks::control`: a `horizon-ctl` command a crate answers, with this window's world.
    pub(super) fn hooks_control(
        &mut self,
        words: &[&str],
    ) -> horizon_client::Result<Option<serde_json::Value>> {
        match horizon_hooks::engine::control(words, &mut World(self)) {
            None => Ok(None),
            Some(Ok(json)) => Ok(Some(serde_json::from_str(&json)?)),
            Some(Err(why)) => Err(why.into()),
        }
    }

    /// A singleplayer world runs on this window's server thread, so it can be shared, as Open
    /// to LAN publishes it; another server can't be.
    pub(super) fn can_share(&self) -> bool {
        self.title.is_none() && (self.remote.is_none() || self.integrated.is_some())
    }
}

pub(super) struct World<'a, 'g>(&'a mut Game<'g>);

impl ShareView for World<'_, '_> {
    fn can_share(&self) -> bool {
        self.0.can_share()
    }

    fn lan_port(&self) -> Option<u16> {
        self.0.lan_port()
    }

    fn open_lan(&mut self, options: LanOptions) -> Result<u16, String> {
        self.0
            .lan_open(LanSettings {
                port: horizon_client::lan::available_port(),
                guest_commands: options.guest_commands,
                force_game_mode: options.force_game_mode,
            })
            .map_err(|e| e.to_string())?;
        self.0
            .lan_port()
            .ok_or_else(|| "the world isn't open to LAN".to_owned())
    }

    fn close_lan(&mut self) {
        self.0.lan_close(true);
    }

    fn players(&self, each: &mut dyn FnMut(&str)) {
        let g = &self.0;
        if let Some(host) = &g.lan {
            for guest in &host.hub.guests {
                each(&guest.name);
            }
        } else if g.integrated.is_some() {
            // On the server thread: the other players the server lists.
            let me = g.options.player_name();
            for name in g.remote_online_names().unwrap_or_default() {
                if name != me {
                    each(&name);
                }
            }
        }
    }

    fn chat(&mut self, text: &str) {
        self.0.system_message(Text::literal(text));
    }

    fn copy(&mut self, text: &str) {
        use horizon_client::clipboard::Clipboard;
        self.0.clipboard.set(text);
    }

    fn open_url(&mut self, url: &str) {
        // The system browser, as the folder buttons open their folders.
        super::packs::open_uri(url);
    }

    fn tr(&self, key: &str) -> Option<String> {
        horizon_client::lang::has(key).then(|| horizon_client::lang::get(key))
    }
}
