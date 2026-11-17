//! `MobLevel` for the game: mob code's level services, forwarded to the game's own.
use super::{
    Game,
    pickups::{Collector, Taken},
};
use horizon_client::{entities::ItemEntity, items::ItemStack};
use horizon_gameplay::mob_level::{MobLevel, MobParts};

impl MobLevel for Game<'_> {
    fn parts(&mut self) -> MobParts<'_> {
        MobParts {
            mobs: &mut self.mobs,
            dropped: &mut self.dropped,
            next_entity: self.next_entity,
            ticked: &self.creatures.ticked,
            items: world_items!(self),
            facts: self.res.facts(),
            world: &mut self.world,
        }
    }
    fn rule(&self, name: &str) -> bool {
        matches!(
            self.ui.rules.get(name),
            Some(horizon_client::gamerules::Value::Bool(true))
        )
    }
    fn mob_griefing(&self) -> bool {
        self.ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING)
    }
    fn entity_grief(&self, entity: u32, rule: bool) -> bool {
        Game::entity_grief(self, entity, rule)
    }
    fn spawn_item_own(&mut self, stack: ItemStack, at: [f64; 3], pickup_delay: u32) -> [f64; 3] {
        Game::spawn_item_own(self, stack, at, pickup_delay)
    }
    fn just_spawned_item(&mut self) -> Option<&mut ItemEntity> {
        Game::just_spawned_item(self)
    }
    fn mob_uuid(&mut self, id: u32) -> [i32; 4] {
        Game::mob_uuid(self, id)
    }
    fn play_sound(&mut self, event: &str, at: [f64; 3], volume: f32, pitch: f32) {
        self.audio.play(event, Some(at), volume, pitch);
    }
    fn on_item_pickup(&mut self, mob: usize, item: &ItemEntity) {
        self.mob_item_pickup_trigger(mob, item);
    }
    fn take(&mut self, collector: u32, item: ItemEntity) {
        self.add_pickup(Taken::Item(Box::new(item)), Collector::Mob(collector));
    }
    fn discard_item(&mut self, index: usize) -> ItemEntity {
        self.remove_dropped(index)
    }
}
