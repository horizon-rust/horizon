//! What mob code calls on `ServerLevel` and `Level`: the level's data as one
//! borrow ([`MobParts`]) and its services, which the game implements once.
use horizon_client::{
    blocks::BlockFacts,
    entities::ItemEntity,
    fast_hash::FxHashSet,
    items::{ItemStack, Items},
    join_seq::JoinList,
    mobs::Mob,
    world::World,
};

/// The level's data as mob code reads it: borrows only, nothing computed.
pub struct MobParts<'a> {
    pub mobs: &'a mut JoinList<Mob>,
    pub dropped: &'a mut JoinList<ItemEntity>,
    /// The id the next entity will take (`ItemAreas` keys its cache on it).
    pub next_entity: u32,
    /// The mobs that ticked this tick.
    pub ticked: &'a FxHashSet<u32>,
    pub items: &'a Items,
    pub facts: &'a BlockFacts,
    pub world: &'a mut World,
}

/// The services `Mob` code calls on its level beyond its data.
pub trait MobLevel {
    fn parts(&mut self) -> MobParts<'_>;
    /// `ServerLevel.getGameRules().get(rule)` for a boolean rule.
    fn rule(&self, name: &str) -> bool;
    /// The `mob_griefing` rule, read without a name lookup.
    fn mob_griefing(&self) -> bool;
    /// Whether a mod lets `entity` grief where `rule` says so
    /// (`entity_mob_griefing`, NeoForge's `canEntityGrief`).
    fn entity_grief(&self, entity: u32, rule: bool) -> bool;
    /// `new ItemEntity(level, x, y, z, stack)` and `addFreshEntity`: the item's
    /// own random draws its motion, which this returns.
    fn spawn_item_own(&mut self, stack: ItemStack, at: [f64; 3], pickup_delay: u32) -> [f64; 3];
    /// The item entity the last spawn added, unless a mod kept it out of the level.
    fn just_spawned_item(&mut self) -> Option<&mut ItemEntity>;
    /// `Entity.getUUID` of a mob, made up for a new one.
    fn mob_uuid(&mut self, id: u32) -> [i32; 4];
    /// `Level.playSound` at a position, no player excluded.
    fn play_sound(&mut self, event: &str, at: [f64; 3], volume: f32, pitch: f32);
    /// `LivingEntity.onItemPickup`: the thrower's advancement trigger.
    fn on_item_pickup(&mut self, mob: usize, item: &ItemEntity);
    /// `LivingEntity.take`: the pickup animation toward the mob `collector`.
    fn take(&mut self, collector: u32, item: ItemEntity);
    /// `Entity.discard` of item entity `index`: it leaves the level.
    fn discard_item(&mut self, index: usize) -> ItemEntity;
}
