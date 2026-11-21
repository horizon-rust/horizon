//! Native worker adapters for API-owned foliage placer types: a mod's `FoliagePlacerType` as the
//! engine's, its placers drawing from the tree's random and writing through the tree's own
//! `tryPlaceLeaf` and foliage setter, with block states converted through the index the server
//! published for its world generation.
use super::native_worldgen::WorkerResources;
use horizon_world::worldgen::mod_foliage_placers::{
    FoliageRandom, ModFoliage, ModFoliageAttachment, ModFoliagePlacer, ModFoliagePlacerType,
    ModFoliagePlacerTypes, install_mod_foliage_placer_types,
};
use mod_api::{
    BlockMetadata, BlockState, FoliageAttachment, FoliageContext, FoliagePlacer, FoliagePlacerType,
    StructureRandom,
};
use std::sync::Arc;

/// The tree's random, as a [`StructureRandom`].
struct Draws<'a>(&'a mut dyn FoliageRandom);
impl StructureRandom for Draws<'_> {
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        self.0.int(bound)
    }
    fn next_long(&mut self) -> i64 {
        self.0.long()
    }
    fn next_float(&mut self) -> f32 {
        self.0.float()
    }
    fn next_double(&mut self) -> f64 {
        self.0.double()
    }
    fn next_boolean(&mut self) -> bool {
        self.0.boolean()
    }
}

/// The engine's canopy, as a [`FoliageContext`].
struct Canopy<'a> {
    inner: &'a mut dyn ModFoliage,
    worker: &'a WorkerResources,
}
impl StructureRandom for Canopy<'_> {
    fn next_int_bound(&mut self, bound: i32) -> i32 {
        self.inner.random().int(bound)
    }
    fn next_long(&mut self) -> i64 {
        self.inner.random().long()
    }
    fn next_float(&mut self) -> f32 {
        self.inner.random().float()
    }
    fn next_double(&mut self) -> f64 {
        self.inner.random().double()
    }
    fn next_boolean(&mut self) -> bool {
        self.inner.random().boolean()
    }
}
impl FoliageContext for Canopy<'_> {
    fn metadata(&self) -> &dyn BlockMetadata {
        self.worker.metadata
    }
    fn random(&mut self) -> &mut dyn StructureRandom {
        self
    }
    fn block(&mut self, pos: [i32; 3]) -> Option<BlockState> {
        let state = self.inner.block(pos)?;
        self.worker.index.to_api(state).ok()
    }
    fn valid_tree_pos(&mut self, pos: [i32; 3]) -> bool {
        self.inner.valid_tree_pos(pos)
    }
    fn try_place_leaf(&mut self, pos: [i32; 3]) -> bool {
        self.inner.try_place_leaf(pos)
    }
    fn set(&mut self, pos: [i32; 3], state: BlockState) {
        match self.worker.index.to_engine(state) {
            Ok(state) => self.inner.set(pos, state),
            Err(_) => {
                log::warn!("Horizon: a mod foliage placer set a block state the server lacks")
            }
        }
    }
    fn is_set(&mut self, pos: [i32; 3]) -> bool {
        self.inner.is_set(pos)
    }
}

/// The engine's placer type around a mod's.
struct TypeAdapter(Arc<dyn FoliagePlacerType>);
impl ModFoliagePlacerType for TypeAdapter {
    fn decode(&self, json: &str) -> Result<Arc<dyn ModFoliagePlacer>, String> {
        Ok(Arc::new(PlacerAdapter(self.0.decode(json)?)))
    }
}

/// The engine's placer around a mod's decoded one.
struct PlacerAdapter(Arc<dyn FoliagePlacer>);
impl ModFoliagePlacer for PlacerAdapter {
    fn foliage_height(&self, random: &mut dyn FoliageRandom, tree_height: i32) -> i32 {
        self.0.foliage_height(&mut Draws(random), tree_height)
    }
    fn foliage_radius(&self, random: &mut dyn FoliageRandom, trunk_height: i32) -> i32 {
        self.0.foliage_radius(&mut Draws(random), trunk_height)
    }
    fn create_foliage(
        &self,
        foliage: &mut dyn ModFoliage,
        tree_height: i32,
        attachment: ModFoliageAttachment,
        foliage_height: i32,
        leaf_radius: i32,
    ) {
        // The states convert through the server's published index, as the worldgen Parts' do.
        let Some(worker) = super::native_worldgen_parts::registry()
            .and_then(|registry| WorkerResources::get(registry).ok())
        else {
            log::warn!("Horizon: a mod foliage placer ran before the server published its blocks");
            return;
        };
        let attachment = FoliageAttachment {
            pos: attachment.pos,
            radius_offset: attachment.radius_offset,
            height_offset: attachment.height_offset,
            size: attachment.size,
        };
        let mut canopy = Canopy {
            inner: foliage,
            worker,
        };
        self.0.create_foliage(
            &mut canopy,
            tree_height,
            attachment,
            foliage_height,
            leaf_radius,
        );
    }
}

fn installed_foliage_placer_types() -> ModFoliagePlacerTypes {
    mod_api::__private::foliage_placer_type_declarations()
        .into_iter()
        .map(|(id, ty)| {
            (
                id,
                Arc::new(TypeAdapter(ty)) as Arc<dyn ModFoliagePlacerType>,
            )
        })
        .collect()
}

/// Hand the foliage placer types the mods registered to the engine's tree features, before any
/// world opens.
pub(in crate::game) fn install() {
    if install_mod_foliage_placer_types(installed_foliage_placer_types()).is_err() {
        log::warn!("Mod foliage placer types already installed; keeping the first table");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::api_values::{block_state, install_parts, resolve_block_definitions};
    use horizon_world::registry::{BlockRegistry, BlockStateId};
    use mod_api::{BlockKey, Hooks, ModEntry, Mods, Registrar, RegistrationContext};
    use std::sync::PoisonError;

    use super::super::native_worldgen_parts::{self, PUBLISHING};

    const LEAVES: BlockStateId = BlockStateId(1);
    const MELON: BlockStateId = BlockStateId(2);
    const FACTS: &str = r#"{"defaults":{"minecraft:air":0,"minecraft:oak_leaves":1,"minecraft:melon":2},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"","minecraft:oak_leaves"],["s",false,false,"","minecraft:melon"]]}"#;

    /// A blob-like canopy with a fruit under its centre: `height` from its JSON plus one draw,
    /// radius 2, one row whose corners a draw skips.
    #[derive(Debug)]
    struct Ring {
        height: i32,
        fruit: BlockKey,
    }
    impl FoliagePlacer for Ring {
        fn foliage_height(&self, random: &mut dyn StructureRandom, _: i32) -> i32 {
            self.height + random.next_int_bound(2)
        }
        fn foliage_radius(&self, _: &mut dyn StructureRandom, _: i32) -> i32 {
            2
        }
        fn create_foliage(
            &self,
            foliage: &mut dyn FoliageContext,
            _: i32,
            attachment: FoliageAttachment,
            _: i32,
            leaf_radius: i32,
        ) {
            mod_api::place_leaves_row(
                foliage,
                attachment.pos,
                leaf_radius,
                0,
                attachment.double_trunk(),
                |random, dx, _, dz, radius, _| {
                    dx == radius && dz == radius && random.next_int_bound(2) == 0
                },
            );
            let [x, y, z] = attachment.pos;
            if foliage.is_set(attachment.pos)
                && let Ok(fruit) = BlockState::of(foliage.metadata(), self.fruit, &[])
            {
                foliage.set([x, y - 1, z], fruit);
            }
        }
    }
    struct RingType(BlockKey);
    impl FoliagePlacerType for RingType {
        fn decode(&self, json: &str) -> Result<Arc<dyn FoliagePlacer>, String> {
            let json: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
            let height = json["height"].as_i64().ok_or("no height")? as i32;
            Ok(Arc::new(Ring {
                height,
                fruit: self.0,
            }))
        }
    }
    struct Orchard;
    #[mod_api::__subscriptions]
    impl Hooks for Orchard {
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            let melon = r.block_key("minecraft:melon");
            r.register_foliage_placer_type("ring", RingType(melon));
        }
    }

    /// The tree's side: every leaf it was asked to place, every block set and every draw.
    #[derive(Default)]
    struct Tree {
        leaves: Vec<[i32; 3]>,
        set: Vec<([i32; 3], BlockStateId)>,
        draws: u32,
    }
    impl FoliageRandom for Tree {
        fn int(&mut self, _: i32) -> i32 {
            self.draws += 1;
            0
        }
        fn long(&mut self) -> i64 {
            self.draws += 1;
            0
        }
        fn float(&mut self) -> f32 {
            self.draws += 1;
            0.0
        }
        fn double(&mut self) -> f64 {
            self.draws += 1;
            0.0
        }
        fn boolean(&mut self) -> bool {
            self.draws += 1;
            false
        }
    }
    impl ModFoliage for Tree {
        fn random(&mut self) -> &mut dyn FoliageRandom {
            self
        }
        fn block(&mut self, _: [i32; 3]) -> Option<BlockStateId> {
            Some(LEAVES)
        }
        fn valid_tree_pos(&mut self, _: [i32; 3]) -> bool {
            true
        }
        fn try_place_leaf(&mut self, pos: [i32; 3]) -> bool {
            self.leaves.push(pos);
            true
        }
        fn set(&mut self, pos: [i32; 3], state: BlockStateId) {
            self.set.push((pos, state));
        }
        fn is_set(&mut self, pos: [i32; 3]) -> bool {
            self.leaves.contains(&pos)
        }
    }

    #[test]
    fn a_mods_placer_draws_from_the_tree_and_writes_through_its_setter() {
        let _publishing = PUBLISHING.lock().unwrap_or_else(PoisonError::into_inner);
        let mods: Mods = vec![ModEntry::new("orchard", Box::new(Orchard))].into();
        mod_platform::testing::with_mods(mods.clone(), || {
            install_parts(&mods).unwrap();
            let registry: &'static BlockRegistry = Box::leak(Box::new(
                BlockRegistry::from_facts_json(FACTS.as_bytes()).unwrap(),
            ));
            resolve_block_definitions(registry).unwrap();
            native_worldgen_parts::publish(None, &block_state::current().unwrap(), registry);
            let types = installed_foliage_placer_types();
            let ring = &types["orchard:ring"];
            assert!(ring.decode(r#"{"type":"orchard:ring"}"#).is_err());
            let placer = ring
                .decode(r#"{"type":"orchard:ring","height":3}"#)
                .unwrap();

            let mut tree = Tree::default();
            assert_eq!(placer.foliage_height(&mut tree, 6), 3);
            assert_eq!(placer.foliage_radius(&mut tree, 3), 2);
            assert_eq!(tree.draws, 1);

            let attachment = ModFoliageAttachment {
                pos: [0, 64, 0],
                radius_offset: 0,
                height_offset: 0,
                size: [1, 1],
            };
            placer.create_foliage(&mut tree, 6, attachment, 3, 2);
            // A 5 by 5 row without the four corners its draws skipped, one draw per corner.
            assert_eq!(tree.leaves.len(), 21);
            assert!(!tree.leaves.contains(&[2, 64, 2]));
            assert_eq!(tree.draws, 5);
            assert_eq!(tree.set, [([0, 63, 0], MELON)]);
        });
    }
}
