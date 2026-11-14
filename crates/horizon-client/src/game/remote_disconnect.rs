//! Leaving a server, with resource packs retained only for a protocol transfer.
use super::*;

impl Game<'_> {
    pub(in crate::game) fn leave_server(&mut self, reason: Option<String>) -> Result<()> {
        self.leave_server_with_transfer(reason, false)
    }

    pub(super) fn leave_server_for_transfer(&mut self) -> Result<()> {
        self.leave_server_with_transfer(None, true)
    }

    /// Disconnect and return to the multiplayer screen (DisconnectedScreen).
    fn leave_server_with_transfer(
        &mut self,
        reason: Option<String>,
        transferring: bool,
    ) -> Result<()> {
        if reason.is_none() && self.integrated.as_ref().is_some_and(|s| !s.stopped()) {
            return self.begin_closing_save(false);
        }
        self.finish_perf();
        if reason.is_none() {
            // Leaving on purpose: nothing for auto-reconnect to retry.
            self.extras_left();
        }
        self.clear_waypoints();
        self.hud_disconnected();
        // Vanilla's Hud also ticks on the title and connect screens, so the
        // action bar has run out by the next world; Horizon's ticks in one.
        self.titles.overlay_time = 0;
        // GameRenderer.resetData and narrator.clear (Minecraft.disconnect).
        self.item_activation.reset();
        // Kept mod block entity geometry belongs to the world leaving.
        self.kept_block_entities.get_mut().clear();
        self.clear_narrator();
        // updateLevelInEngines(null): every sound stops and the music manager
        // lets go of its track. Leaving a server comes straight here, not
        // through finish_close_world, so the biome's ambient loop would
        // otherwise play on over the title screen until the next world.
        self.ambience.clear(&mut self.audio);
        self.music.stop_playing(None, &mut self.audio);
        self.audio.stop_all();
        // A world is leaving either way: its icon attempt goes with it.
        self.world_icon = None;
        // Minecraft.disconnect: the client level goes.
        if let Some(old) = self.remote.as_mut().and_then(|r| r.heard_level.take()) {
            self.native_client_level_unload(&old);
        }
        if self.remote.is_some() {
            self.native_client_disconnecting();
        }
        if let Some(mut r) = self.remote.take() {
            r.session.disconnect();
        }
        crate::game::lan::forget_sync_data();
        self.gui.atlas.clear_object_skins();
        self.server_packs_disconnect(transferring);
        self.code_of_conduct = None;
        self.reset_tutorial();
        self.close_dialogs();
        // Gui.setScreen -> KeyMapping.releaseAll: the title or disconnect
        // screen releases every held key. Its release goes to that screen,
        // so a key kept here would still be down in the next world (#3246).
        self.keys.clear();
        self.left = false;
        self.right = false;
        // The shared world is closing: its invite code ends with it.
        self.friends_lan_closed();
        // Single player's server thread saves the world before the title shows.
        let single_player = match self.integrated.take() {
            Some(server) => {
                server.stop()?;
                true
            }
            None => false,
        };
        // A splitscreen player leaving says nothing of where P1 plays.
        if !self.options.split_guest {
            horizon_client::crash::clear_detail("Server");
        }
        self.clock = Default::default();
        self.adv = Default::default();
        self.stats = Default::default();
        self.score.board = Default::default();
        self.streaming = Streaming::new(Arc::new(NoTerrain), self.worker_context(), 1);
        self.streaming.paused = true;
        self.world = World::new(self.res.facts().air, 0);
        self.player = None;
        // The vehicle went with the level: an id kept here would seat the
        // next login on an entity that is not there.
        self.riding = None;
        // Out of the server's bed: InBedChatScreen goes with the level.
        self.sleeping = None;
        self.sleep_timer = 0;
        self.clear_entities();
        self.score.board = Default::default();
        self.tab_list = Default::default();
        self.border = Default::default();
        self.screen = Screen::Playing;
        self.container = None;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections(self.world.layout());
        }
        let saves = std::env::current_dir()?.join("saves");
        let mut title = horizon_client::title::Title::new(saves.clone(), &self.options.resources)
            .with_user(|| self.options.player_name());
        title.last_server = self.settings.file.string("lastServer");
        title.skip_multiplayer_warning = self.settings.file.bool("skipMultiplayerWarning");
        if single_player && reason.is_none() {
            self.title = Some(title);
            self.capture(false);
            return Ok(());
        }
        title.mode = horizon_client::title::Mode::Multiplayer(match reason {
            Some(reason) => horizon_client::multiplayer::Screen::Disconnected {
                reason,
                retry: None,
                lines: std::cell::Cell::new(1),
            },
            None => horizon_client::multiplayer::Screen::list(&saves),
        });
        self.title = Some(title);
        self.capture(false);
        Ok(())
    }
}
