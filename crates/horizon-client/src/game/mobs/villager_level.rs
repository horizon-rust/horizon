//! `VillagerLevel` for the game: villager code's level services, forwarded to the game's own.
use super::Game;
use horizon_client::{
    items::{ItemStack, Items},
    mobs::{MobType, MobWorld, Seen, spawning::BiomeFacts},
    stations::StationData,
};
use horizon_core::position::BlockPos;
use horizon_gameplay::villager_level::VillagerLevel;
use std::{collections::HashSet, sync::Arc};

impl VillagerLevel for Game<'_> {
    fn items(&self) -> &Items {
        Game::items(self)
    }
    fn stations(&self) -> &StationData {
        Game::stations(self)
    }
    fn villager_picks_up(&self) -> Option<HashSet<String>> {
        self.res
            .items()
            .item_tags
            .get("minecraft:villager_picks_up")
            .cloned()
    }
    fn mob_type(&self, id: &str) -> Option<Arc<MobType>> {
        self.res.mob_types().get(id).cloned()
    }
    fn mob_world_seen<'s>(&'s self, others: &'s [Seen]) -> MobWorld<'s> {
        self.mob_world_base(others)
    }
    fn set_next_entity(&mut self, id: u32) {
        self.next_entity = id;
    }
    fn mob_joined(&mut self, from: usize, natural: bool) {
        self.native_mobs_joined(from, natural);
    }
    fn biome_facts(&self, x: i32, y: i32, z: i32) -> BiomeFacts {
        Game::biome_facts(self, x, y, z)
    }
    fn spawn_mob_raw(&mut self, kind: &str, at: [f64; 3], persistent: bool) -> Option<u32> {
        Game::spawn_mob_raw(self, kind, at, persistent)
    }
    fn mob_event_particles(&mut self, id: u32, event: i8) {
        Game::mob_event_particles(self, id, event);
    }
    fn bonemeal_grow(&mut self, pos: BlockPos, stack: &mut ItemStack) -> (bool, Option<bool>) {
        let ask = self.bonemeal(pos, false, stack);
        let grown = ask.crop.unwrap_or_else(|| {
            self.with_level(|b, level, ticks| b.grow_crop(level, ticks, pos, ask.check))
        });
        (grown, ask.crop)
    }
    fn level_event(&mut self, event: horizon_world::level::LevelEvent) {
        self.level_events.push(event);
        let _ = self.process_level_events();
    }
    fn composter_level(&self, pos: [i32; 3]) -> Option<i32> {
        Game::composter_level(self, pos)
    }
    fn compost_items(&mut self, mob: usize, pos: [i32; 3]) {
        self.villager_compost_items(mob, pos);
    }
    fn hurt_by_player_uuid(&self, id: u32) -> Option<[i32; 4]> {
        self.last_hurt_by_player.get(&id).and_then(|credit| {
            self.resolve_player_credit(*credit)
                .and_then(|who| self.player_uuid_of(who))
        })
    }
    fn player_uuid_of(&self, who: Option<u32>) -> Option<[i32; 4]> {
        Game::player_uuid_of(self, who)
    }
    fn lan_acting(&self) -> Option<u32> {
        self.lan_acting
    }
    fn wake_villager(&mut self, id: u32) {
        Game::wake_villager(self, id);
    }
}
