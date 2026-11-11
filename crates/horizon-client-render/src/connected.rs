//! Connected textures at mesh time: a pack format vanilla doesn't read (OptiFine's `optifine/ctm`,
//! Fusion's connecting models) picks another sprite for a face, or one per quarter, from the
//! blocks around it, and overlays to draw over it. The platform's pack formats fill it
//! ([`crate::pack_hooks`]) once per pack load, with their rules and a meshing entry point
//! ([`Connected::mesh`]) that runs the mesher on their own type: the mesher calls
//! [`ConnectedTextures::pick`] statically, never through a trait object, per face. Without
//! connected textures the mesher runs on [`NoConnected`] and nothing changes.

use std::any::Any;
use std::num::NonZeroU16;
use std::sync::Arc;

use crate::chunk::Chunk;
use crate::mesher::{MeshScratch, Mesher, SectionMesh};

/// A tile as the mesher draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileLayer {
    Layer {
        layer: u16,
        /// The sprite has fractional alpha (drawn in the translucent pass).
        translucent: bool,
        /// Every texel is opaque (drawn without an alpha test).
        opaque: bool,
    },
    /// Try the next rule.
    Skip,
    /// The face's own sprite.
    Default,
}

/// What a face draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// Its own sprite.
    Keep,
    Tile(TileLayer),
    /// A tile per quarter of the sprite (top-left, top-right, bottom-left, bottom-right).
    Quarters([TileLayer; 4]),
    /// Nothing but its overlays.
    Nothing,
}

/// A tile drawn over a face, after its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overlay {
    pub layer: u16,
    /// Drawn in the translucent pass, else with an alpha cutout.
    pub translucent: bool,
    /// The tint index and the state whose tint it is (else the face's block).
    pub tint: Option<(i32, Option<u32>)>,
}

/// The face being meshed.
#[derive(Clone, Copy, Debug)]
pub struct Face<'a> {
    /// Its block state.
    pub state: u32,
    /// Its sprite layer.
    pub layer: u16,
    /// The side it faces, in `horizon_assets::model::Direction` order (down, up, north, south,
    /// west, east).
    pub facing: u8,
    /// The block's position in the world.
    pub pos: [i32; 3],
    /// The quad's corners (block-local) and their UVs.
    pub positions: &'a [[f32; 3]; 4],
    pub uv: &'a [[f32; 2]; 4],
    /// The block is a glass pane, whose tiles are drawn with the cube's UVs.
    pub pane: bool,
    pub tint_index: i32,
    /// The rule a pack format gave this face of the baked model
    /// (`crate::models::BakedFace::fusion`).
    pub rule: Option<NonZeroU16>,
}

/// A pack's connected textures, as the mesher asks them.
pub trait ConnectedTextures: Send + Sync + 'static {
    /// No rule at all: the mesher skips the stacked pane test.
    fn is_empty(&self) -> bool;

    /// Whether a pane's top or bottom strip, centred at `centre` (block-local x, z), is hidden by
    /// the pane `other` stacked on it.
    fn stacked_pane_hides(&self, state: u32, other: u32, centre: [f32; 2]) -> bool;

    /// What `face` draws. `neighbour` answers the state at an offset from the block; `covers`
    /// whether a state hides the face of a block beside it; `biome` the block's biome, asked
    /// only by rules that test one. Overlays go to `overlay`.
    fn pick<'b>(
        &self,
        face: &Face,
        neighbour: impl Fn([i32; 3]) -> u32,
        covers: impl Fn(u32) -> bool,
        biome: impl Fn() -> Option<&'b str>,
        overlay: impl FnMut(Overlay),
    ) -> Pick;
}

/// No connected textures.
pub struct NoConnected;

impl ConnectedTextures for NoConnected {
    fn is_empty(&self) -> bool {
        true
    }

    fn stacked_pane_hides(&self, _: u32, _: u32, _: [f32; 2]) -> bool {
        false
    }

    fn pick<'b>(
        &self,
        _: &Face,
        _: impl Fn([i32; 3]) -> u32,
        _: impl Fn(u32) -> bool,
        _: impl Fn() -> Option<&'b str>,
        _: impl FnMut(Overlay),
    ) -> Pick {
        Pick::Keep
    }
}

/// [`Mesher::mesh_reusing`] for a pack with connected textures: the format's own entry point,
/// which calls [`Mesher::mesh_with`] on its own [`ConnectedTextures`] type (`rules`, downcast).
pub type MeshFn = fn(
    &mut Mesher<'_>,
    &(dyn Any + Send + Sync),
    &[[Option<&Chunk>; 3]; 3],
    i32,
    SectionMesh,
    &mut MeshScratch,
) -> SectionMesh;

/// A pack's connected textures as the renderer keeps them.
pub struct Connected {
    /// The format's rules: its [`ConnectedTextures`] type.
    pub rules: Arc<dyn Any + Send + Sync>,
    pub mesh: MeshFn,
}
