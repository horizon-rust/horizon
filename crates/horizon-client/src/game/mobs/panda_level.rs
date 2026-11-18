//! `PandaLevel` for the game: the food tag, the click's game mode and the love and
//! held-stack steps panda code asks of the game.
use super::Game;
use horizon_gameplay::panda_level::PandaLevel;

impl PandaLevel for Game<'_> {
    fn panda_eats_from_ground(&self, item: &str) -> bool {
        self.res
            .items()
            .item_tags
            .get("minecraft:panda_eats_from_ground")
            .is_some_and(|tag| tag.contains(item))
    }
    fn is_creative(&self) -> bool {
        self.creative
    }
    fn panda_in_love(&mut self, id: u32) {
        self.love_cause.insert(id, self.lan_acting);
        let kind = self
            .mobs
            .iter()
            .find(|m| m.id == id)
            .map(super::native_hooks::mob_kind)
            .unwrap_or_default();
        self.native_entity_enter_love_mode(id, &kind, self.lan_acting_uuid);
        // Animal.setInLove: entity event 18, the love hearts.
        self.mob_event_particles(id, 18);
    }
    fn consume_held(&mut self) {
        Game::consume_held(self);
    }
    fn after_mobs_lifted(&mut self) {
        #[cfg(test)]
        super::entity::taken_pass_tests::after_take(self);
    }
}
