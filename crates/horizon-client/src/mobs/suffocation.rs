//! Runtime binding of Blocks' suffocation definitions to the native registry.
use horizon_world::{
    block_collision::BlockCollisionRegistry,
    suffocation::{SuffocationDefinition, SuffocationRegistry},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::Arc,
};

/// The predicates installed by Blocks' registrations. Predicate evaluation,
/// including full-shape tests and block entities, stays in SuffocationRegistry.
pub fn bind<'r>(
    collision: Arc<BlockCollisionRegistry<'r>>,
    causes_suffocation: &BTreeSet<String>,
) -> io::Result<SuffocationRegistry<'r>> {
    use SuffocationDefinition::*;
    let registry = collision.registry();
    let mut definitions = BTreeMap::new();
    for i in 0..registry.len() {
        let state = registry
            .state(horizon_world::registry::BlockStateId(i as u32))
            .unwrap();
        if definitions.contains_key(&state.name) {
            continue;
        }
        let definition = match state.name.strip_prefix("minecraft:") {
            Some("dirt_path" | "farmland" | "mud" | "soul_sand") => Always,
            Some("piston" | "sticky_piston") => NotExtendedPiston,
            Some("mangrove_roots" | "moving_piston") => Never,
            // `ShulkerBoxBlock` suffocates while its lid is shut. What is asked here
            // is the block's *block entity type* (`BlockEntityType.validBlocks`:
            // `minecraft:shulker_box` for vanilla's boxes), which a content block
            // carries when its declared type says so — not its class, which the
            // client's own suffocation rule reads (`game::entities::cushions`).
            // The arm is before the name suffixes below, so a block
            // whose id ends in a suffix answers for its type, not for the suffix.
            _ if crate::containers::is_shulker_box_block(&state.name) => ClosedShulker,
            Some(name)
                if name.ends_with("_leaves")
                    || name == "glass"
                    || name == "tinted_glass"
                    || name.ends_with("_stained_glass")
                    || name == "copper_grate"
                    || name.ends_with("_copper_grate") =>
            {
                Never
            }
            _ => TaggedFullCollision,
        };
        definitions.insert(state.name.clone(), definition);
    }
    SuffocationRegistry::new(collision, &definitions, causes_suffocation)
}
