use crate::game::Game;
use horizon_client::{
    command::Context,
    server::user_lists::{NameAndId, WhiteEntry},
    text::Text,
};
use horizon_gameplay::commands::{host::R, profiles::ProfilesHost};

impl ProfilesHost for Game<'_> {
    fn require_dedicated(&mut self) -> R<()> {
        Game::dedicated_lists(self).map(|_| ())
    }
    fn request_stop(&mut self) {
        if let Ok(lists) = Game::dedicated_lists(self) {
            lists.lock().unwrap().stop = true;
        }
    }
    fn entity_save_admitted(&self) -> bool {
        Game::admit_entity_save().is_ok()
    }
    fn guest_turn_active(&self) -> bool {
        self.lan_acting.is_some()
    }
    fn save_after_guest_turn(&mut self) {
        self.save_after_guest_turn = true;
    }
    fn save_game(&mut self) -> Result<(), String> {
        Game::save_game(self).map_err(|e| e.to_string())
    }
    fn set_auto_save(&mut self, enabled: bool) -> bool {
        match Game::dedicated_lists(self) {
            Ok(lists) => crate::game::dedicated::set_auto_save(&lists, enabled),
            Err(_) => false,
        }
    }
    fn set_player_idle_timeout(&mut self, minutes: i32) {
        if let Ok(lists) = Game::dedicated_lists(self) {
            Game::set_player_idle_timeout(self, &lists, minutes);
        }
    }
    fn reason_message(&self, reason: Option<&str>) -> Text {
        Game::reason_message(reason)
    }
    fn ban_entries(&self, id: &str) -> Vec<(String, String, Option<String>)> {
        let Ok(lists) = Game::dedicated_lists(self) else {
            return Vec::new();
        };
        let l = lists.lock().unwrap();
        let players = l
            .bans
            .entries()
            .into_iter()
            .map(|b| (b.user.name.clone(), b.source.clone(), b.reason.clone()));
        let ips = l
            .ip_bans
            .entries()
            .into_iter()
            .map(|b| (b.user.clone(), b.source.clone(), b.reason.clone()));
        match id {
            "banlist.ips" => ips.collect(),
            "banlist.players" => players.collect(),
            _ => players.chain(ips).collect(),
        }
    }
    fn reload_whitelist(&mut self) {
        if let Ok(lists) = Game::dedicated_lists(self) {
            // PlayerList.reloadWhiteList.
            let _ = lists.lock().unwrap().white.load();
        }
    }
    fn using_whitelist(&self) -> bool {
        match Game::dedicated_lists(self) {
            Ok(lists) => lists.lock().unwrap().use_whitelist,
            Err(_) => false,
        }
    }
    fn set_using_whitelist(&mut self, on: bool) {
        if let Ok(lists) = Game::dedicated_lists(self) {
            Game::set_using_whitelist(self, &lists, on);
        }
    }
    fn kick_unlisted_players(&mut self) {
        if let Ok(lists) = Game::dedicated_lists(self) {
            Game::kick_unlisted_players(self, &lists);
        }
    }
    fn whitelist_names(&self) -> Vec<String> {
        let Ok(lists) = Game::dedicated_lists(self) else {
            return Vec::new();
        };
        lists
            .lock()
            .unwrap()
            .white
            .entries()
            .iter()
            .map(|e| e.user.name.clone())
            .collect()
    }
    fn whitelist_apply(&mut self, add: bool, target: &NameAndId) -> bool {
        let Ok(lists) = Game::dedicated_lists(self) else {
            return false;
        };
        let mut l = lists.lock().unwrap();
        if add {
            !l.white.contains(target)
                && l.white.add(WhiteEntry {
                    user: target.clone(),
                })
        } else {
            l.white.contains(target) && l.white.remove(target)
        }
    }
    fn game_profiles(&mut self, ctx: &Context, name: &str) -> R<Vec<NameAndId>> {
        Game::game_profiles(self, ctx, name)
    }
}
