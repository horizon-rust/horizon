//! Custom entity models over Horizon's mob model layers: a pack format vanilla doesn't read
//! (OptiFine's `.jem` files, Entity Model Features') reshapes a layer (`zombie#main`), maybe
//! paints it with its own texture and animates it after the vanilla animation. The platform's
//! formats build them once per pack load ([`install`]); a mob's draw only swaps the layer's part
//! tree and runs its [`LayerAnimation`]. Without them every model stays vanilla's.

use super::anim::{Poses, State};
use super::{Mob, MobType, Part};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, OnceLock, PoisonError, RwLock},
};

/// The custom models of the enabled packs, by model layer (`zombie#main`).
#[derive(Default)]
pub struct CemModels {
    layers: HashMap<String, CemLayer>,
}

/// A model layer as a pack reshapes it.
pub struct CemLayer {
    pub model: Arc<Part>,
    /// The pack's animations, run after the vanilla animation.
    pub animation: Option<Box<dyn LayerAnimation>>,
    /// The atlas name of the pack's texture for the layer (`entity/zombie/zombie_green`),
    /// drawn instead of the layer's vanilla texture.
    pub texture: Option<String>,
    /// The item anchors the pack's bones declare (`right_handheld_item`, `fox_item`): the
    /// bone's part name, the anchor and its offset.
    pub anchors: Vec<(String, String, [f32; 3])>,
}

/// A layer's animation: poses `poses` (already posed by the vanilla animation) for `m` in
/// state `s`. Called once per draw of an animated layer.
pub trait LayerAnimation: Send + Sync {
    fn run(&self, m: &Mob, s: &State, poses: &mut Poses);
}

/// What builds a pack's custom entity models: the platform's pack formats.
pub trait ModelFormats: Send + Sync {
    /// The models the overlay at `root` gives the layers of `types`, and the species
    /// (`MobType::id`) whose models read the block a mob stands on ([`tracks_standing`]).
    fn entity_models(
        &self,
        root: &Path,
        types: &HashMap<String, Arc<MobType>>,
    ) -> (CemModels, Vec<String>);
}

static FORMATS: OnceLock<&'static dyn ModelFormats> = OnceLock::new();

/// Installs the model formats, before the first pack loads. The first call wins.
pub fn install(formats: &'static dyn ModelFormats) {
    let _ = FORMATS.set(formats);
}

impl CemModels {
    /// `layers` by model layer key.
    pub fn new(layers: HashMap<String, CemLayer>) -> Self {
        Self { layers }
    }

    /// The custom models of the overlay at `root` over the layers of `types`; none without
    /// installed formats.
    pub fn load(root: &Path, types: &HashMap<String, Arc<MobType>>) -> Self {
        let (models, standing) = FORMATS
            .get()
            .map_or_else(Default::default, |f| f.entity_models(root, types));
        *STANDING.write().unwrap_or_else(PoisonError::into_inner) =
            Some(standing.into_iter().collect());
        models
    }

    pub fn layer(&self, key: &str) -> Option<&CemLayer> {
        self.layers.get(key)
    }
}

/// The species whose models read the standing block: only their mobs pay the per-tick
/// lookup, and none does without such a pack. Keyed like `MobType::id` (`minecraft:cow`).
static STANDING: RwLock<Option<HashSet<String>>> = RwLock::new(None);

/// Whether the species' models read the block under the mob's feet each tick (the window's
/// remote tick keeps `standing_on` fresh; the server, which draws nothing, never looks).
pub fn tracks_standing(id: &str) -> bool {
    STANDING
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .is_some_and(|s| s.contains(id))
}
