//! Which block textures are thin: lit from behind, the light of the sun comes out the other side
//! (RT lighting's subsurface scattering, `gfx/parallax.rs`). Leaves, plants, vines, flowers,
//! grass and crops are thin; every other block is thick, and the sun does not go through it.
//!
//! The sprite arrays know textures, not blocks, so the answer is per sprite layer: a layer is
//! thin when some thin block's model draws it (`thin_layers`), and the Parallax shader reads it
//! from a buffer by the fragment's layer (`gfx/materials.rs`). A resource pack's own word wins
//! where it has any: a specular map's subsurface scattering (`labpbr.rs`) is honoured on any
//! block, thin or thick, and this list answers where the maps are silent.

use crate::assets::RenderAssets;
use crate::blocks::BlockFacts;
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;

/// Whether the block named `name` (no namespace) of the classes `has_class` tells is thin: the
/// classes `mesher.rs`'s `ray_shape` calls cutouts, and the growing plants, flower beds, leaf
/// litter and dry plants it has no need to tell apart.
pub fn is_thin(name: &str, has_class: impl Fn(&str) -> bool) -> bool {
    [
        "LeavesBlock",
        "VegetationBlock",
        "VineBlock",
        "BaseCoralPlantTypeBlock",
        "GrowingPlantBlock",
        "FlowerBedBlock",
        "LeafLitterBlock",
        "DryVegetationBlock",
    ]
    .iter()
    .any(|class| has_class(class))
        || name.ends_with("_vines")
        || name.ends_with("_roots")
}

/// Whether the block named `name` (no namespace) burns with a flame: its emitting texels are a
/// black body's (terrain.wgsl `rt_flame`), where a lamp's or a froglight's keep their own colour.
pub fn is_flame(name: &str) -> bool {
    // A sea lantern is a lamp of prismarine, and a fire coral a coral: neither burns.
    !(name.starts_with("sea_") || name.contains("coral"))
        && ["torch", "fire", "campfire", "candle", "lantern"]
            .iter()
            .any(|part| name.ends_with(part) || name.contains(&format!("{part}_")))
}

/// What kind of flame the block named `name` (no namespace) burns, for the heat haze above it
/// (`gfx/haze.rs`): one of `Flame`'s kinds, 0 for a block that is no flame or gives no heat (a
/// redstone torch burns cold).
pub fn flame_kind(name: &str) -> u8 {
    use super::haze::Flame;
    if !is_flame(name) || name.contains("redstone") {
        0
    } else if name.ends_with("campfire") {
        Flame::CAMPFIRE
    } else if name.ends_with("fire") {
        Flame::FIRE
    } else if name.contains("candle") {
        Flame::CANDLE
    } else if name.contains("lantern") {
        Flame::LANTERN
    } else {
        Flame::TORCH
    }
}

/// Whether the sprite `name` (`minecraft:block/fire_0`) is a flame's that the shader draws
/// itself (terrain.wgsl `rt_fire`): the sprites of fire, soul fire and the campfires, not a
/// torch's tiny flame nor the campfires' logs.
fn is_procedural_sprite(name: &str) -> bool {
    let name = name.rsplit('/').next().unwrap_or(name);
    matches!(
        name,
        "fire_0"
            | "fire_1"
            | "soul_fire_0"
            | "soul_fire_1"
            | "campfire_fire"
            | "soul_campfire_fire"
    )
}

/// The blocks' thin textures and flames, by sprite layer (`layers` of them): bit 0 for a layer
/// that a thin block's model draws, bit 1 for one a flame's model draws, bit 2 for the sprite of
/// a fire the shader draws procedurally.
pub fn thin_layers(facts: &BlockFacts, render: &RenderAssets, layers: usize) -> Vec<u32> {
    let mut out = vec![0u32; layers];
    for (name, layer) in &render.sprite_layers {
        if is_procedural_sprite(name)
            && let Some(bits) = out.get_mut(*layer as usize)
        {
            *bits |= 4;
        }
    }
    let mut models = Vec::new();
    for (id, s) in facts.states.iter().enumerate() {
        let name = s.name.strip_prefix("minecraft:").unwrap_or(&s.name);
        let bit = if is_thin(name, |class| s.has_class(class)) {
            1
        } else if is_flame(name) {
            2
        } else {
            continue;
        };
        // A block's model may vary with its position: look at a few of them.
        for (x, z) in [(0, 0), (1, 0), (0, 1), (1, 1), (2, 3), (5, 7)] {
            let at = BlockPos::new(x, 0, z);
            if render
                .model_catalog
                .models_at(BlockStateId(id as u32), at, &mut models)
                .is_err()
            {
                break;
            }
            for face in models.iter().flat_map(|m| &m.faces) {
                if let Some(thin) = out.get_mut(face.layer as usize) {
                    *thin |= bit;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flames_have_their_kinds_and_the_lamps_and_the_cold_have_none() {
        use super::super::haze::Flame;
        assert_eq!(flame_kind("torch"), Flame::TORCH);
        assert_eq!(flame_kind("soul_wall_torch"), Flame::TORCH);
        assert_eq!(flame_kind("fire"), Flame::FIRE);
        assert_eq!(flame_kind("soul_fire"), Flame::FIRE);
        assert_eq!(flame_kind("soul_campfire"), Flame::CAMPFIRE);
        assert_eq!(flame_kind("white_candle"), Flame::CANDLE);
        assert_eq!(flame_kind("candle_cake"), Flame::CANDLE);
        assert_eq!(flame_kind("soul_lantern"), Flame::LANTERN);
        assert_eq!(flame_kind("redstone_torch"), 0);
        assert_eq!(flame_kind("sea_lantern"), 0);
        assert_eq!(flame_kind("fire_coral"), 0);
        assert_eq!(flame_kind("stone"), 0);
    }

    #[test]
    fn leaves_plants_and_vines_are_thin_and_stone_and_planks_are_not() {
        let thin = |name: &str, classes: &[&str]| is_thin(name, |c| classes.contains(&c));
        assert!(thin("oak_leaves", &["Block", "LeavesBlock"]));
        assert!(thin(
            "short_grass",
            &["Block", "VegetationBlock", "TallGrassBlock"]
        ));
        assert!(thin("vine", &["VineBlock"]));
        assert!(thin("tube_coral", &["BaseCoralPlantTypeBlock"]));
        assert!(thin("kelp", &["GrowingPlantBlock"]));
        assert!(thin("weeping_vines_plant", &["GrowingPlantBlock"]));
        assert!(thin("hanging_roots", &[]));
        assert!(!thin("stone", &["Block"]));
        assert!(!thin("oak_planks", &["Block"]));
        assert!(!thin("cactus", &["CactusBlock"]));
        assert!(!thin("hay_block", &["HayBlock"]));
    }
}
