//! The resource pack formats vanilla doesn't read (OptiFine's, Polytone's) on the engine's
//! extension points ([`crate::pack_hooks`]). This is platform code (docs/layers.md) kept in this
//! crate until the platform's executable installs it: it uses only the crate's public API, so it
//! moves to a platform crate unchanged, and nothing else in the crate names it.

pub mod cit;
mod cit_glint;
mod colors;
mod connected;
mod ctm;
mod custom_sky;
mod fusion;
mod item_bars;

use std::path::Path;
use std::sync::Arc;

use crate::pack_hooks::PackFormats;

/// OptiFine's and Polytone's formats.
pub struct Formats;

impl PackFormats for Formats {
    fn colors(&self, root: &Path) -> Option<crate::pack_palette::PackColors> {
        Some(colors::read(root))
    }

    fn prepare_connected_textures(
        &self,
        root: &Path,
        sprites: &mut crate::sprites::Sprites,
    ) -> Option<Box<dyn std::any::Any + Send + Sync>> {
        let rules = horizon_optifine::ctm::load(root);
        sprites.keep_apart(ctm::ConnectedTextures::matched_sprites(&rules));
        Some(Box::new(rules))
    }

    fn connected_textures(
        &self,
        root: &Path,
        prepared: Option<Box<dyn std::any::Any + Send + Sync>>,
        registry: &horizon_world::registry::BlockRegistry,
        sprites: &mut crate::sprites::Sprites,
        models: &mut crate::models::StateModels,
        native_mods: &mut dyn FnMut(&mut crate::sprites::Sprites),
    ) -> Option<crate::connected::Connected> {
        let rules = *prepared?
            .downcast::<Vec<horizon_optifine::ctm::Rule>>()
            .expect("the connected rules this format prepared");
        connected::load(root, rules, registry, sprites, models, native_mods)
    }

    fn emissive_suffix(&self, root: &Path) -> Option<String> {
        Some(horizon_optifine::emissive::suffix(root))
    }

    fn item_textures(&self, root: &Path) -> Option<crate::cit::CitRules> {
        Some(cit::load(root))
    }

    fn custom_sky(&self, root: &Path) -> Option<Arc<dyn crate::custom_sky::SkyLayers>> {
        let sky = custom_sky::CustomSky::load(root);
        (!sky.is_empty()).then(|| Arc::new(sky) as _)
    }

    fn item_bars(&self, root: &Path) -> Option<Arc<dyn crate::pack_palette::ItemBars>> {
        item_bars::read(root)
    }
}

/// Installs these formats ([`crate::pack_hooks::install`]); the game does at startup.
pub fn install() {
    crate::pack_hooks::install(&Formats);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connected::{ConnectedTextures, Face, Pick, TileLayer};
    use crate::sprites::Sprites;
    use std::fs;

    #[test]
    fn prepared_rules_isolate_named_sprites_and_are_used_without_rereading() {
        let root = std::env::temp_dir().join(format!(
            "pack-connected-prepare-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let textures = root.join("assets/minecraft/textures/block");
        let ctm = root.join("assets/minecraft/optifine/ctm/fixture");
        fs::create_dir_all(&textures).unwrap();
        fs::create_dir_all(&ctm).unwrap();
        let write = |path, pixels: &[u8]| {
            let mut encoder = png::Encoder::new(fs::File::create(path).unwrap(), 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(pixels)
                .unwrap();
        };
        for name in ["placeholder", "named", "again"] {
            write(textures.join(format!("{name}.png")), &[0; 16 * 16 * 4]);
        }
        write(ctm.join("0.png"), &[255; 16 * 16 * 4]);
        let file = ctm.join("test.properties");
        fs::write(&file, "matchTiles=block/named\nmethod=fixed\ntiles=0").unwrap();
        let mut sprites = Sprites::default();
        let prepared = Formats.prepare_connected_textures(&root, &mut sprites);
        let placeholder = sprites
            .insert(&root, "minecraft:block/placeholder")
            .unwrap();
        let named = sprites.insert(&root, "minecraft:block/named").unwrap();
        let again = sprites.insert(&root, "minecraft:block/again").unwrap();
        assert_ne!(named, placeholder);
        assert_eq!(again, placeholder);
        fs::remove_file(file).unwrap();
        let registry = horizon_world::registry::BlockRegistry::from_facts_json(
            br#"{"defaults":{"minecraft:stone":0},"states":[["stone",false,false,"","minecraft:stone"]]}"#.as_slice(),
        ).unwrap();
        let connected = Formats
            .connected_textures(
                &root,
                prepared,
                &registry,
                &mut sprites,
                &mut crate::models::StateModels::default(),
                &mut |_| {},
            )
            .unwrap();
        let rules = connected.rules.downcast_ref::<connected::Rules>().unwrap();
        let face = Face {
            state: 0,
            layer: named as u16,
            facing: 1,
            pos: [0, 64, 0],
            positions: &[[0.0; 3]; 4],
            uv: &[[0.0; 2]; 4],
            pane: false,
            tint_index: -1,
            rule: None,
        };
        let pick = |face: &Face| rules.pick(face, |_| 0, |_| false, || None, |_| {});
        assert_eq!(
            pick(&Face {
                layer: placeholder as u16,
                ..face
            }),
            Pick::Keep
        );
        assert_eq!(
            pick(&face),
            Pick::Tile(TileLayer::Layer {
                layer: sprites.layers["assets/minecraft/optifine/ctm/fixture/0.png"] as u16,
                translucent: false,
                opaque: true,
            })
        );
        fs::remove_dir_all(root).unwrap();
    }
}
