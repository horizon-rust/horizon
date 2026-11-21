//! Foliage placer types a mod registers (vanilla's `FoliagePlacerType` with its own
//! `FoliagePlacer`), handed over once at startup and looked up when a tree feature's
//! `foliage_placer` names one (`{"type": "modid:fruit"}`). The rest of the tree stays the
//! engine's: a mod placer draws from the tree's random and writes its canopy through the tree's
//! own `tryPlaceLeaf` and foliage setter. Trees place on the worldgen worker threads and on the
//! server thread, so a placer keeps no mutable state.
use crate::registry::BlockStateId;
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

/// A foliage placer type's engine side, as the executable's adapter sees it.
pub trait ModFoliagePlacerType: Send + Sync {
    /// The tree feature's `foliage_placer` object, as JSON text, into its placer; `Err` refuses
    /// the feature.
    fn decode(&self, json: &str) -> Result<Arc<dyn ModFoliagePlacer>, String>;
}

/// A decoded placer: vanilla's `FoliagePlacer`, asked in vanilla's order.
pub trait ModFoliagePlacer: Send + Sync {
    /// `foliageHeight(random, treeHeight, tree)`.
    fn foliage_height(&self, random: &mut dyn FoliageRandom, tree_height: i32) -> i32;
    /// `foliageRadius(random, trunkHeight)`.
    fn foliage_radius(&self, random: &mut dyn FoliageRandom, trunk_height: i32) -> i32;
    /// The public `createFoliage`: the placer samples its own offset, then places.
    fn create_foliage(
        &self,
        foliage: &mut dyn ModFoliage,
        tree_height: i32,
        attachment: ModFoliageAttachment,
        foliage_height: i32,
        leaf_radius: i32,
    );
}

/// `FoliagePlacer.FoliageAttachment`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModFoliageAttachment {
    pub pos: [i32; 3],
    pub radius_offset: i32,
    pub height_offset: i32,
    pub size: [i32; 2],
}

/// The tree's random. A `next_int` bound below 1, which vanilla refuses with an exception, draws
/// nothing and answers 0; the tree then fails as it fails on one of its own.
pub trait FoliageRandom {
    fn int(&mut self, bound: i32) -> i32;
    fn long(&mut self) -> i64;
    fn float(&mut self) -> f32;
    fn double(&mut self) -> f64;
    fn boolean(&mut self) -> bool;
}

/// What a placer writes its canopy through. A world error ends the tree once the placer returns;
/// until then reads answer `None` or false and writes are dropped.
pub trait ModFoliage {
    fn random(&mut self) -> &mut dyn FoliageRandom;
    /// `WorldGenLevel.getBlockState`.
    fn block(&mut self, pos: [i32; 3]) -> Option<BlockStateId>;
    /// `TreeFeature.validTreePos`.
    fn valid_tree_pos(&mut self, pos: [i32; 3]) -> bool;
    /// `FoliagePlacer.tryPlaceLeaf` with the tree's foliage provider.
    fn try_place_leaf(&mut self, pos: [i32; 3]) -> bool;
    /// `FoliageSetter.set`; a state the registry doesn't hold is dropped.
    fn set(&mut self, pos: [i32; 3], state: BlockStateId);
    /// `FoliageSetter.isSet`.
    fn is_set(&mut self, pos: [i32; 3]) -> bool;
}

/// The installed foliage placer types, `modid:fruit` → type. Set once, before any world opens
/// (the executable hands the mods' registrations over).
pub type ModFoliagePlacerTypes = BTreeMap<String, Arc<dyn ModFoliagePlacerType>>;

static MOD_FOLIAGE_PLACER_TYPES: OnceLock<ModFoliagePlacerTypes> = OnceLock::new();
static EMPTY: ModFoliagePlacerTypes = BTreeMap::new();

#[cfg(test)]
thread_local! {
    /// The types a test installs, in place of the process-wide table.
    static TEST_TYPES: std::cell::Cell<Option<&'static ModFoliagePlacerTypes>> =
        const { std::cell::Cell::new(None) };
}

/// Install the process-wide table once; a second installation keeps the first.
pub fn install_mod_foliage_placer_types(
    types: ModFoliagePlacerTypes,
) -> Result<(), ModFoliagePlacerTypes> {
    MOD_FOLIAGE_PLACER_TYPES.set(types)
}

/// The installed foliage placer types: empty until the executable installs the mods', and a
/// test's own while one is installed.
pub fn mod_foliage_placer_types() -> &'static ModFoliagePlacerTypes {
    #[cfg(test)]
    if let Some(types) = TEST_TYPES.get() {
        return types;
    }
    MOD_FOLIAGE_PLACER_TYPES.get().unwrap_or(&EMPTY)
}

#[cfg(test)]
/// Runs `with` over a test's own types, as the tree decoder reads them.
pub(crate) fn with_test_types<T>(types: &ModFoliagePlacerTypes, with: impl FnOnce() -> T) -> T {
    let leaked: &'static ModFoliagePlacerTypes = Box::leak(Box::new(types.clone()));
    let prev = TEST_TYPES.replace(Some(leaked));
    let out = with();
    TEST_TYPES.set(prev);
    out
}
