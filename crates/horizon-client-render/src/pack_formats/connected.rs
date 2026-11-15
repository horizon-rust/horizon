//! OptiFine's and Fusion's connected textures as the mesher draws them
//! ([`crate::connected::ConnectedTextures`]): a Fusion face first, then OptiFine's rules.

use std::path::Path;
use std::sync::Arc;

use horizon_optifine::ctm::connect::{Axes, Face};
use horizon_world::registry::{BlockRegistry, BlockStateId};

use super::ctm::ConnectedTextures as Ctm;
use super::fusion::FusionTextures;
use crate::chunk::Chunk;
use crate::connected::{self as engine, Connected, ConnectedTextures, Overlay, Pick};
use crate::mesher::{MeshScratch, Mesher, SectionMesh};
use crate::models::StateModels;
use crate::sprites::Sprites;

/// A pack's Fusion and OptiFine connected textures.
pub struct Rules {
    ctm: Ctm,
    fusion: FusionTextures,
}

/// The connected textures under `root`: Fusion's models' tiles, native mods' sprites, then
/// OptiFine's rules' tiles go into `sprites`, in that order. `None` without either format's files.
pub fn load(
    root: &Path,
    rules: Vec<horizon_optifine::ctm::Rule>,
    registry: &BlockRegistry,
    sprites: &mut Sprites,
    models: &mut StateModels,
    native_mods: &mut dyn FnMut(&mut Sprites),
) -> Option<Connected> {
    let fusion = {
        let _step = crate::spans::startup::step("core.fusion");
        FusionTextures::load(root, registry, sprites, models)
    };
    native_mods(sprites);
    // A side's sprite, for rules that connect by tile: the first quad facing it.
    let catalog = &*models;
    let mut found = Vec::new();
    let ctm = Ctm::load(
        root,
        rules,
        registry,
        sprites,
        |state: BlockStateId, face: Face| {
            let side = horizon_assets::model::Direction::ALL[face as usize];
            catalog
                .models_at(
                    state,
                    horizon_core::position::BlockPos::new(0, 0, 0),
                    &mut found,
                )
                .ok()?;
            found
                .iter()
                .flat_map(|m| &m.faces)
                .find(|f| f.quad.facing == side)
                .map(|f| f.layer)
        },
    );
    let rules = Rules { ctm, fusion };
    (!rules.is_empty()).then(|| Connected {
        rules: Arc::new(rules),
        mesh,
    })
}

/// The mesher on [`Rules`].
fn mesh(
    mesher: &mut Mesher<'_>,
    rules: &(dyn std::any::Any + Send + Sync),
    region: &[[Option<&Chunk>; 3]; 3],
    sy: i32,
    out: SectionMesh,
    scratch: &mut MeshScratch,
) -> SectionMesh {
    let rules = rules
        .downcast_ref::<Rules>()
        .expect("the connected textures this format loaded");
    mesher.mesh_with(rules, region, sy, out, scratch)
}

impl ConnectedTextures for Rules {
    fn is_empty(&self) -> bool {
        self.ctm.is_empty() && self.fusion.is_empty()
    }

    fn stacked_pane_hides(&self, state: u32, other: u32, centre: [f32; 2]) -> bool {
        !self.ctm.is_empty() && self.ctm.stacked_pane_hides(state, other, centre)
    }

    fn pick<'b>(
        &self,
        face: &engine::Face,
        neighbour: impl Fn([i32; 3]) -> u32,
        covers: impl Fn(u32) -> bool,
        biome: impl Fn() -> Option<&'b str>,
        mut overlay: impl FnMut(Overlay),
    ) -> Pick {
        let side = Face::ALL[usize::from(face.facing)];
        let fused = face.rule.and_then(|rule| {
            self.fusion.pick(
                rule,
                face.state,
                side,
                face.tint_index,
                &neighbour,
                covers,
                &mut overlay,
            )
        });
        if let Some(pick) = fused {
            pick
        } else if self.ctm.is_empty() {
            Pick::Keep
        } else {
            self.ctm.pick(
                face.state,
                face.layer,
                side,
                face.pos,
                &neighbour,
                biome,
                // A pane's tiles are drawn with the cube's UVs, so that is its texture.
                || {
                    if face.pane {
                        Some(Axes::cube(side))
                    } else {
                        Axes::texture(face.positions, face.uv)
                    }
                },
                &mut overlay,
            )
        }
    }
}
