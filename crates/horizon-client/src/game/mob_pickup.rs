//! `Mob.aiStep`'s looting for the mobs that pick up whole items: a mob that
//! can pick up loot takes the item entities within its reach that it wants
//! (`Mob.wantsToPickUp`), and `Mob.pickUpItem` puts them where they belong.
//! Villagers, piglins, pandas, allays and sulfur cubes have their own loops.
use super::{
    Game,
    dispenser::new_item_motion,
    pickups::{Collector, Taken},
};
use horizon_client::{
    entities::ItemEntity,
    mobs::{Species, loot},
};
use horizon_core::{geometry::Vec3, random::RandomSource};

/// The mobs whose pickups have their own code (`villager_pickups`,
/// `piglin_pickups`, `tick_pandas`, `tick_allays`, `sulfur_cube_pickups`).
fn picks_up_elsewhere(species: Species) -> bool {
    matches!(
        species,
        Species::Villager
            | Species::Piglin
            | Species::PiglinBrute
            | Species::Panda
            | Species::Allay
            | Species::SulfurCube
            | Species::Fox
            | Species::Dolphin
            | Species::Pillager
    )
}

impl Game<'_> {
    /// `Mob.aiStep`: every mob that can pick up loot, with `mob_griefing` on,
    /// goes through the item entities in its box grown by the pickup reach
    /// (`getPickupReach`, 1 block sideways), and picks up the ones that have
    /// no pickup delay and that it wants.
    pub(super) fn tick_mob_pickups(&mut self) {
        if !self.ui.rules.bool("mob_griefing") {
            return;
        }
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if !m.alive() || !m.can_pick_up_loot() || picks_up_elsewhere(m.species()) {
                continue;
            }
            let reach = m.bounds().inflate(Vec3::new(1.0, 0.0, 1.0));
            // The entities are listed first: a pickup can remove some.
            let found: Vec<u32> = self
                .dropped
                .iter()
                .filter(|e| e.stack.count > 0 && reach.intersects(e.bounds()))
                .map(|e| e.id)
                .collect();
            for id in found {
                let Some(j) = self.dropped.iter().position(|e| e.id == id) else {
                    continue;
                };
                let e = &self.dropped[j];
                if e.pickup_delay > 0
                    || !loot::wants_to_pick_up(&self.mobs[i], self.res.items(), &e.stack.item)
                {
                    continue;
                }
                self.equip_picked_up_item(i, id);
            }
        }
    }
    /// `Mob.pickUpItem`: the mob equips the item entity's stack where it
    /// belongs (`equipItemIfPossible`); what it takes leaves the entity, which
    /// is removed when nothing is left.
    fn equip_picked_up_item(&mut self, i: usize, id: u32) {
        let Some(j) = self.dropped.iter().position(|e| e.id == id) else {
            return;
        };
        let entity = self.dropped[j].clone();
        let Some(equipped) =
            loot::equip_item_if_possible(&mut self.mobs[i], self.res.items(), &entity.stack)
        else {
            return;
        };
        // equipItemIfPossible: the item it replaces falls at its feet.
        if let Some(old) = equipped.replaced {
            let m = &self.mobs[i];
            let at = [m.position.x, m.position.y, m.position.z];
            self.spawn_item(old, at, new_item_motion(), 10);
        }
        // setItemSlot -> onEquipItem: an item put on in the slot its equippable
        // names plays its equip sound (the seed is drawn from the mob's random).
        if equipped.worn && !self.mobs[i].silent {
            let sound = self
                .res
                .items()
                .get(&entity.stack.item)
                .and_then(|info| info.equip_sound.clone())
                .unwrap_or_else(|| "minecraft:item.armor.equip_generic".into());
            let m = &mut self.mobs[i];
            let _ = m.random.next_long();
            let at = [m.position.x, m.position.y, m.position.z];
            self.audio.play(&sound, Some(at), 1.0, 1.0);
        }
        self.mob_item_picked_up(i, &entity);
        self.mob_takes_item(i, j, equipped.taken.count);
    }
    /// `LivingEntity.onItemPickup`: the player who threw the item gets
    /// `thrown_item_picked_up_by_entity` for the mob that took it.
    pub(super) fn mob_item_picked_up(&mut self, i: usize, entity: &ItemEntity) {
        if entity
            .thrower
            .is_some_and(|uuid| Some(uuid) == self.level.player_uuid)
        {
            let target = self.mob_entity(&self.mobs[i]);
            self.trigger_item_entity(
                "minecraft:thrown_item_picked_up_by_entity",
                &entity.stack,
                &target,
            );
        }
    }
    /// `LivingEntity.take` and `ItemStack.shrink`: the item flies into the
    /// mob (`ClientboundTakeItemEntityPacket`, which the clients draw and
    /// play the pickup sound for) and `count` of its stack are gone; an empty
    /// entity is discarded.
    pub(super) fn mob_takes_item(&mut self, i: usize, j: usize, count: u32) {
        let entity = self.dropped[j].clone();
        let mob = self.mobs[i].id;
        if let Some(host) = self.lan.as_ref() {
            let at = [entity.position.x, entity.position.y, entity.position.z];
            host.hub
                .take_item(entity.id as i32, mob as i32, count as i32, at);
        }
        self.add_pickup(Taken::Item(Box::new(entity)), Collector::Mob(mob));
        let left = &mut self.dropped[j];
        left.stack.count = left.stack.count.saturating_sub(count);
        if left.stack.count == 0 {
            self.dropped.remove(j);
        }
    }
}
