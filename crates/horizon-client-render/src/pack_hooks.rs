//! The engine's extension points for resource pack formats vanilla doesn't read (OptiFine's,
//! Polytone's: platform code, docs/layers.md). The engine asks [`formats`] once per pack load and
//! gets engine types back (tables, or an object it calls at most once a frame), never a format's
//! own types. The platform installs its formats at startup ([`install`]); with none installed,
//! packs load as vanilla reads them.

use std::path::Path;
use std::sync::{Arc, OnceLock};

/// What the pack formats beyond vanilla's add to a pack. Each method reads one overlay of a pack
/// (its folder at `root`); the default is "nothing in this format".
pub trait PackFormats: Send + Sync {
    /// The colours the overlay replaces where vanilla hardcodes them.
    fn colors(&self, _root: &Path) -> Option<crate::pack_palette::PackColors> {
        None
    }

    /// The name ending of the overlay's emissive textures (`stone_e.png` beside `stone.png`),
    /// if its format has them.
    fn emissive_suffix(&self, _root: &Path) -> Option<String> {
        None
    }

    /// The overlay's connected textures: their tiles go into `sprites` (before it finishes), and
    /// baked faces they draw on may get a rule (`crate::models::BakedFace::fusion`).
    /// `native_mods` loads native mods' own sprites: call it after the tiles that go before them.
    fn connected_textures(
        &self,
        _root: &Path,
        _registry: &horizon_world::registry::BlockRegistry,
        _sprites: &mut crate::sprites::Sprites,
        _models: &mut crate::models::StateModels,
        _native_mods: &mut dyn FnMut(&mut crate::sprites::Sprites),
    ) -> Option<crate::connected::Connected> {
        None
    }

    /// The overlay's custom item texture rules and their matcher.
    fn item_textures(&self, _root: &Path) -> Option<crate::cit::CitRules> {
        None
    }

    /// The overlay's entity texture variants, blinks and emissive overlays.
    fn entity_textures(&self, _root: &Path) -> Option<crate::entity_textures::EntityTextures> {
        None
    }

    /// The block entity models the overlay reshapes over `vanilla`'s, with their textures;
    /// `None` when it reshapes none.
    fn block_entity_models(
        &self,
        _root: &Path,
        _vanilla: &std::collections::HashMap<&'static str, crate::block_entity_render::ModelPart>,
    ) -> Option<Reshaped> {
        None
    }

    /// The sky layers the overlay draws over the vanilla sky, if it has any.
    fn custom_sky(&self, _root: &Path) -> Option<Arc<dyn crate::custom_sky::SkyLayers>> {
        None
    }

    /// The overlay's changes to the dimensions' and biomes' environment attributes.
    fn environment_modifiers(
        &self,
        _root: &Path,
    ) -> Option<Box<dyn crate::environment::EnvironmentModifiers>> {
        None
    }

    /// The overlay's block and fluid modifiers that tint with a colormap.
    fn colormap_targets(&self, _root: &Path) -> Vec<crate::environment::ColormapTarget> {
        Vec::new()
    }

    /// The overlay's colormap `id` (`namespace:path`), as blocks sample it.
    fn block_colormap(
        &self,
        _root: &Path,
        _id: &str,
    ) -> Option<Arc<dyn crate::environment::BlockColormap>> {
        None
    }

    /// The post chains of the packs merged at `root`, with the packs' option values; `None`
    /// without any.
    fn post_chains(
        &self,
        _root: &Path,
        _options: horizon_pack_options::Configs,
    ) -> Option<Box<dyn crate::polytone_post::PostChains>> {
        None
    }

    /// `source` compiled as an expression over the frame (an animation `mode`); `None` when the
    /// formats have no such expressions or it doesn't compile.
    fn frame_expression(
        &self,
        _source: &str,
    ) -> Option<Box<dyn crate::environment::FrameExpression>> {
        None
    }

    /// The screen sprite modifiers of the packs merged at `root`, for the GUI sprites among
    /// `keys` (the atlas's).
    fn sprite_modifiers(
        &self,
        _root: &Path,
        _keys: &mut dyn Iterator<Item = &String>,
    ) -> Option<crate::hud::sprite_modifiers::SpriteModifiers> {
        None
    }

    /// The creative tab modifiers of the packs merged at `root`, in load order.
    fn creative_tab_modifiers(&self, _root: &Path) -> Vec<Box<dyn TabModifier>> {
        Vec::new()
    }

    /// The colormaps the overlay puts in place of vanilla's biome colour functions, given its
    /// `modifiers` ([`Self::colormap_targets`]).
    fn biome_colormaps(
        &self,
        _root: &Path,
        _modifiers: &[crate::environment::ColormapTarget],
    ) -> Vec<(
        crate::environment::BiomeFunction,
        Arc<dyn crate::environment::BlockColormap>,
    )> {
        Vec::new()
    }

    /// The custom particle types and emitters of the packs merged at `root`, with the block
    /// states their block emitters aim at; `None` when no pack has any.
    fn custom_particles(
        &self,
        _root: &Path,
        _blocks: &'static horizon_world::registry::BlockRegistry,
    ) -> Option<Arc<dyn crate::particles::custom::CustomParticles>> {
        None
    }

    /// The options the pack (its base, not its overlays) declares, at their defaults.
    fn pack_options(
        &self,
        _source: &crate::packs::Source,
    ) -> Option<horizon_pack_options::Configs> {
        None
    }

    /// Whether an `overlays.entries[]` item of `pack.mcmeta` is active, given the pack's options;
    /// `formats_apply` is vanilla's answer from its `formats`.
    fn overlay_active(
        &self,
        _entry: &serde_json::Value,
        _options: &horizon_pack_options::Configs,
        formats_apply: bool,
    ) -> bool {
        formats_apply
    }

    /// A pack's `sounds.json` without the events its options turn off; `None` when it keeps
    /// them all.
    fn sounds_without_unmet(
        &self,
        _text: &[u8],
        _options: &horizon_pack_options::Configs,
    ) -> Option<Vec<u8>> {
        None
    }

    /// The durability bar colours the overlay sets by item, if any.
    fn item_bars(&self, _root: &Path) -> Option<Arc<dyn crate::pack_palette::ItemBars>> {
        None
    }
}

/// Every block entity model, reshaped by a pack or vanilla's, and the atlas names of the
/// pack's textures by model ([`PackFormats::block_entity_models`]).
pub type Reshaped = (
    std::collections::HashMap<&'static str, crate::block_entity_render::ModelPart>,
    std::collections::HashMap<&'static str, String>,
);

/// A pack's change to creative tabs: items removed and added, another icon or name.
pub trait TabModifier: Send + Sync {
    /// Whether it changes the tab `id` (`minecraft:building_blocks`).
    fn aims_at(&self, tab: &str) -> bool;

    /// The tab's item ids after its removals and additions. `in_tag(item, tag)`: whether an
    /// item is in an item tag.
    fn apply(&self, items: &mut Vec<String>, in_tag: &dyn Fn(&str, &str) -> bool);

    /// The tab's new icon, an item id.
    fn icon(&self) -> Option<&str>;

    /// The tab's new name, a text component.
    fn name(&self) -> Option<&serde_json::Value>;
}

/// No format beyond vanilla's.
struct Vanilla;

impl PackFormats for Vanilla {}

static FORMATS: OnceLock<&'static dyn PackFormats> = OnceLock::new();

/// Installs the pack formats, before the first pack loads. The first call wins: later ones are
/// ignored, so each test or probe that needs the formats may install them.
pub fn install(formats: &'static dyn PackFormats) {
    let _ = FORMATS.set(formats);
}

/// The installed pack formats, or vanilla's alone.
pub fn formats() -> &'static dyn PackFormats {
    FORMATS.get().copied().unwrap_or(&Vanilla)
}
