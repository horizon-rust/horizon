//! Foliage placer types a trait mod registers: vanilla's `FoliagePlacerType` with its own
//! `FoliagePlacer`, as
//! [`Registrar::register_foliage_placer_type`](crate::Registrar::register_foliage_placer_type)
//! hands it over once at startup (NeoForge's registry `FOLIAGE_PLACER_TYPE`). A tree feature
//! with `"foliage_placer": {"type": "modid:fruit"}` then grows its canopy by the mod's code, in
//! world generation and from a sapling alike.
//!
//! There is no `Host` here: trees place on the worldgen worker threads and on the server thread,
//! so a placer sees only its [`FoliageContext`], keeps no mutable state and is deterministic, the
//! same random giving the same canopy. The rest of the tree stays the game's own: the trunk, the
//! roots, the decorators and the leaves' distances.
use crate::wire::{Reader, Wire, WireError, Writer};
use crate::{BlockMetadata, BlockState, StructureRandom};
use std::sync::Arc;

/// Where a trunk wants foliage (`FoliagePlacer.FoliageAttachment`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla's record, which the game and a mod's own trunk logic both build
#[derive(horizon_mod_api_macros::Wire)]
pub struct FoliageAttachment {
    /// `pos`: the block the canopy centres on, in world blocks.
    pub pos: [i32; 3],
    /// `radiusOffsetXZ`: added to the leaf radius by the trunk (a branch's smaller crown).
    pub radius_offset: i32,
    /// `foliageHeightOffset`: added to the foliage height by the trunk.
    pub height_offset: i32,
    /// `sizeX` and `sizeZ`: 2 by 2 under a two-by-two trunk, else 1 by 1.
    pub size: [i32; 2],
}
impl FoliageAttachment {
    /// An attachment at `pos`, as a trunk placer makes one.
    pub fn new(pos: [i32; 3], radius_offset: i32, double_trunk: bool) -> Self {
        let size = if double_trunk { 2 } else { 1 };
        Self {
            pos,
            radius_offset,
            height_offset: 0,
            size: [size; 2],
        }
    }
    /// `doubleTrunk`: the canopy sits on a two-by-two trunk.
    pub fn double_trunk(&self) -> bool {
        self.size == [2, 2]
    }
}

/// A mod's foliage placer type: vanilla's `FoliagePlacerType`, its codec. Implemented once, kept
/// by the game.
pub trait FoliagePlacerType: Send + Sync + 'static {
    /// The codec: the placer the tree feature's `foliage_placer` object describes (as JSON text,
    /// its `type` included), decoded once when the feature loads. An `Err` refuses the feature,
    /// naming the type and the message, as a data pack's bad field does.
    fn decode(&self, json: &str) -> Result<Arc<dyn FoliagePlacer>, String>;
}
crate::wire::wire_handlers::handler!(dyn FoliagePlacerType, "FoliagePlacerType");

/// One decoded foliage placer: vanilla's `FoliagePlacer`. The tree asks it in vanilla's order,
/// [`foliage_height`](Self::foliage_height), then [`foliage_radius`](Self::foliage_radius) before
/// its trunk, then [`create_foliage`](Self::create_foliage) for each attachment the trunk left,
/// all drawing from the tree's one random.
pub trait FoliagePlacer: std::fmt::Debug + Send + Sync {
    /// `foliageHeight(random, treeHeight, tree)`: the canopy's height for a tree `tree_height`
    /// blocks tall.
    fn foliage_height(&self, random: &mut dyn StructureRandom, tree_height: i32) -> i32;
    /// `foliageRadius(random, trunkHeight)`: the leaf radius; vanilla's samples the placer's
    /// `radius`.
    fn foliage_radius(&self, random: &mut dyn StructureRandom, trunk_height: i32) -> i32;
    /// The public `createFoliage(level, setter, random, tree, treeHeight, attachment,
    /// foliageHeight, leafRadius)`: sample the placer's `offset` first, as vanilla's does, then
    /// place the canopy at `attachment` through `foliage`. [`place_leaves_row`] is vanilla's
    /// `placeLeavesRow` around the placer's own `shouldSkipLocation`.
    fn create_foliage(
        &self,
        foliage: &mut dyn FoliageContext,
        tree_height: i32,
        attachment: FoliageAttachment,
        foliage_height: i32,
        leaf_radius: i32,
    );
}
crate::wire::wire_handlers::handler!(dyn FoliagePlacer, "FoliagePlacer");

/// What a placer's [`create_foliage`](FoliagePlacer::create_foliage) reaches: the level the tree
/// grows in, its random, its `FoliageSetter` and the tree's own `tryPlaceLeaf`.
pub trait FoliageContext {
    /// Immutable block metadata for this worker's resource snapshot.
    fn metadata(&self) -> &dyn BlockMetadata;
    /// The tree's random (`RandomSource` of `createFoliage`).
    fn random(&mut self) -> &mut dyn StructureRandom;
    /// The block at `pos` (`WorldGenLevel.getBlockState`), or `None` where the tree can't read.
    fn block(&mut self, pos: [i32; 3]) -> Option<BlockState>;
    /// `TreeFeature.validTreePos`: whether `pos` is air or `#minecraft:replaceable_by_trees`.
    fn valid_tree_pos(&mut self, pos: [i32; 3]) -> bool;
    /// `FoliagePlacer.tryPlaceLeaf`: where `pos` holds no persistent leaves and is a valid tree
    /// position, the tree's foliage provider's state (waterlogged in water) through the setter.
    /// True when it placed one.
    fn try_place_leaf(&mut self, pos: [i32; 3]) -> bool;
    /// `FoliageSetter.set`: write `state` at `pos` as foliage, which the tree's decorators and its
    /// leaves' distances then count.
    fn set(&mut self, pos: [i32; 3], state: BlockState);
    /// `FoliageSetter.isSet`: whether this tree's foliage already holds `pos`.
    fn is_set(&mut self, pos: [i32; 3]) -> bool;
}

/// `FoliagePlacer.placeLeavesRow`: one row of leaves `y` above `origin`, `radius` around it (one
/// wider on the positive sides over a double trunk), each position through `try_place_leaf`
/// unless `skip` (the placer's `shouldSkipLocation`, given vanilla's `shouldSkipLocationSigned`
/// distances: the random, `dx`, `y`, `dz`, the radius and the double trunk) skips it.
pub fn place_leaves_row(
    foliage: &mut dyn FoliageContext,
    origin: [i32; 3],
    radius: i32,
    y: i32,
    double_trunk: bool,
    mut skip: impl FnMut(&mut dyn StructureRandom, i32, i32, i32, i32, bool) -> bool,
) {
    let offset = i32::from(double_trunk);
    let mut dx = radius.wrapping_neg();
    while dx <= radius.wrapping_add(offset) {
        let mut dz = radius.wrapping_neg();
        while dz <= radius.wrapping_add(offset) {
            let (mx, mz) = if double_trunk {
                (
                    dx.wrapping_abs().min(dx.wrapping_sub(1).wrapping_abs()),
                    dz.wrapping_abs().min(dz.wrapping_sub(1).wrapping_abs()),
                )
            } else {
                (dx.wrapping_abs(), dz.wrapping_abs())
            };
            if !skip(foliage.random(), mx, y, mz, radius, double_trunk) {
                let pos = [
                    origin[0].wrapping_add(dx),
                    origin[1].wrapping_add(y),
                    origin[2].wrapping_add(dz),
                ];
                foliage.try_place_leaf(pos);
            }
            dz += 1;
        }
        dx += 1;
    }
}

/// A registered foliage placer type; the game builds its engine adapter at startup.
pub(crate) struct FoliagePlacerTypeInfo {
    pub(crate) id: Box<str>,
    pub(crate) ty: Arc<dyn FoliagePlacerType>,
}
/// Registered foliage placer types, for the hidden game integration boundary.
// hidden: the game's startup adapter reads the registered types; mods declare them through the registrar.
#[doc(hidden)]
pub fn foliage_placer_type_declarations() -> Vec<(String, Arc<dyn FoliagePlacerType>)> {
    crate::parts::installed_foliage_placer_type_list()
        .iter()
        .map(|info| (info.id.to_string(), info.ty.clone()))
        .collect()
}
