//! What piglin code calls on `ServerLevel` beyond [`MobLevel`]: the content tables it
//! reads and the questions only the game can answer.
use crate::mob_level::MobLevel;
use horizon_client::{
    items::{ItemStack, Items},
    mobs::{
        Mob,
        equip::{Facts, Slot},
    },
};
use horizon_core::geometry::Vec3;

pub trait PiglinLevel: MobLevel {
    /// The game's own content tables (`Resources.items`): item tags and item data.
    fn res_items(&self) -> &Items;
    /// The content tables the world plays with (its data packs' over the game's own).
    fn world_items(&self) -> &Items;
    /// The numbers `Mob.compareArmor` and `compareWeapons` read off `stack` in `slot`
    /// for the mob `m`.
    fn stack_facts(&self, m: &Mob, stack: &ItemStack, slot: Slot) -> Facts;
    /// `PiglinAi.isBarterCurrency` after vanilla's own answer (`currency`): what the
    /// parts of the stack's item and the mods that listen make of it.
    fn piglin_currency_hook(&self, stack: &ItemStack, currency: bool) -> bool;
    /// Where `PiglinAi.throwItems` throws toward from `at`: one block above the feet of
    /// the nearest player within 16 blocks, the level's own player ahead of the guests,
    /// those alive and not spectators. `None` when no player qualifies.
    fn piglin_throw_target(&self, at: Vec3) -> Option<[f64; 3]>;
}
