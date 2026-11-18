//! `PiglinLevel` for the game: the tables and questions piglin code asks beyond `MobLevel`.
use super::Game;
use horizon_client::{
    items::{ItemStack, Items},
    mobs::{
        Mob,
        equip::{Facts, Slot},
    },
};
use horizon_gameplay::piglin_level::PiglinLevel;

impl PiglinLevel for Game<'_> {
    fn res_items(&self) -> &Items {
        self.res.items()
    }
    fn world_items(&self) -> &Items {
        self.items()
    }
    fn stack_facts(&self, m: &Mob, stack: &ItemStack, slot: Slot) -> Facts {
        Game::stack_facts(self, m, stack, slot)
    }
    fn piglin_currency_hook(&self, stack: &ItemStack, mut currency: bool) -> bool {
        if mod_api::__private::item_listens(mod_api::HookId::IsPiglinCurrency, &stack.item) {
            self.native_is_piglin_currency(stack, &mut currency);
        }
        currency
    }
    fn piglin_player_positions(&self) -> Vec<[f64; 3]> {
        let host = self
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying() && !self.spectator())
            .map(|p| [p.position.x, p.position.y, p.position.z]);
        let guests = self.lan.iter().flat_map(|h| {
            h.hub
                .guests
                .iter()
                .filter(|g| h.hub.here(g) && g.loaded && !g.dead && !g.spectator())
                .map(|g| g.position)
        });
        host.into_iter().chain(guests).collect()
    }
}
