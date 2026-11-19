// Adapted from Starling (https://github.com/starling-client/starling, `android` branch at
// 150a414b09a1, `fit_mobile_atlas` in `crates/horizon-client-render/src/hud.rs`), written by the
// Starling contributors. Copyright (c) 2026 Ludovic Marie and the Horizon contributors, under the
// same MIT license as Horizon (LICENSE at the repository root).

//! The shared atlas when a pack's sprites don't fit the GPU's limit (or `MAX_ATLAS_SIZE`, the
//! largest side Horizon lays out): once the custom item textures are left out and the model
//! sprites reduced, as for any pack too large for the atlas, the larger sprites are halved until
//! the atlas fits, instead of the pack failing to load.

use crate::Result;
use crate::hud::{HudAtlas, MAX_ATLAS_SIZE, PackedImage};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU32, Ordering};

/// The GPU's `max_texture_dimension_2d` once a renderer has opened its device.
static TEXTURE_LIMIT: AtomicU32 = AtomicU32::new(MAX_ATLAS_SIZE);

/// Pack the shared atlases built from now on within `limit` texels a side (`Renderer::new`, with
/// the device's `max_texture_dimension_2d`); `limit` back.
pub(crate) fn note_limit(limit: u32) -> u32 {
    TEXTURE_LIMIT.store(limit, Ordering::Relaxed);
    limit
}

/// The largest atlas side `HudAtlas::pack` lays out: the GPU's limit, at most `MAX_ATLAS_SIZE`.
pub(crate) fn limit() -> u32 {
    TEXTURE_LIMIT.load(Ordering::Relaxed).min(MAX_ATLAS_SIZE)
}

/// The fallback for a pack that doesn't fit `limit` without its custom item textures:
/// `HudAtlas::layout_reducing_models`, and only when even that doesn't fit, `layout_halving`. A
/// pack that fits once its model sprites are reduced keeps every other sprite whole, as before.
pub(crate) fn layout_reducing(
    images: &mut [PackedImage],
    gui_sizes: &mut HashMap<String, (u32, u32)>,
    (models, bordered): (&HashSet<String>, &HashSet<String>),
    limit: u32,
) -> Result<(u32, Vec<(u32, u32)>, u32)> {
    HudAtlas::layout_reducing_models(images, gui_sizes, models, limit)
        .or_else(|_| layout_halving(images, gui_sizes, bordered, limit))
}

/// Halve the larger sprites until `images` (the blurred ones, `bordered`, with their border of one
/// texel) fit `limit` texels a side. Fonts, GUI sprites, Horizon's own sprites and run-time slots,
/// and sprites of 32x32 or less keep their size. A halved sprite keeps its size before halving
/// (without the border) in `gui_sizes`, so it draws at the same size from fewer texels, and a
/// bordered one wraps its border again from its halved edges. The last layout's error when
/// nothing is left to halve.
fn layout_halving(
    images: &mut [PackedImage],
    gui_sizes: &mut HashMap<String, (u32, u32)>,
    bordered: &HashSet<String>,
    limit: u32,
) -> Result<(u32, Vec<(u32, u32)>, u32)> {
    loop {
        let error = match HudAtlas::layout(images, limit) {
            Ok(layout) => return Ok(layout),
            Err(error) => error,
        };
        let mut halved = false;
        for image in images.iter_mut() {
            let (name, stride, rgba) = (&image.0, image.1, &image.3);
            let b = u32::from(bordered.contains(name));
            let (w, h) = (image.1 - 2 * b, image.2 - 2 * b);
            if ["font/", "gui/", "ui/", "horizon/"]
                .iter()
                .any(|p| name.starts_with(p))
                || rgba.is_empty()
                || (w <= 32 && h <= 32)
            {
                continue;
            }
            gui_sizes.entry(name.clone()).or_insert((w, h));
            let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
            // Nearest texel of the sprite inside its border.
            let mut smaller = Vec::with_capacity((nw * nh * 4) as usize);
            for y in 0..nh {
                for x in 0..nw {
                    let at = (((b + y * h / nh) * stride + b + x * w / nw) * 4) as usize;
                    smaller.extend_from_slice(&rgba[at..at + 4]);
                }
            }
            let smaller = (name.clone(), nw, nh, smaller);
            *image = match b {
                1 => crate::hud::wrap_border(&smaller),
                _ => smaller,
            };
            halved = true;
        }
        if !halved {
            return Err(error);
        }
        images.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
        log::info!("halved the larger atlas sprites to fit a {limit}x{limit} atlas");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::{GLYPH_ROWS, wrap_border};

    fn solid(name: &str, side: u32, value: u8) -> PackedImage {
        (
            name.into(),
            side,
            side,
            vec![value; (side * side * 4) as usize],
        )
    }

    fn sorted(mut images: Vec<PackedImage>) -> Vec<PackedImage> {
        images.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
        images
    }

    fn find<'a>(images: &'a [PackedImage], name: &str) -> &'a PackedImage {
        images.iter().find(|i| i.0 == name).unwrap()
    }

    /// A 256x256 blurred particle whose texels hold their column, with its border.
    fn bordered_particle() -> PackedImage {
        let rgba = (0..256 * 256).flat_map(|i| [(i % 256) as u8; 4]).collect();
        wrap_border(&("particle/big".into(), 256, 256, rgba))
    }

    #[test]
    fn a_pack_that_fits_with_its_models_reduced_keeps_every_other_sprite() {
        // 24 item sprites of 1024x1024 overflow a 4096x4096 atlas; halved by
        // `layout_reducing_models`, they fit with the entity and the particle at full size.
        let mut images: Vec<PackedImage> = (0..24)
            .map(|i| solid(&format!("item/sword{i}"), 1024, 1))
            .collect();
        images.push(solid("entity/zombie", 1024, 2));
        images.push(solid("particle/flame", 64, 3));
        images.push(bordered_particle());
        let mut images = sorted(images);
        assert!(HudAtlas::layout(&images, 4096).is_err());
        let models: HashSet<String> = (0..24).map(|i| format!("item/sword{i}")).collect();
        let bordered = HashSet::from(["particle/big".to_owned()]);
        let mut sizes = HashMap::new();
        layout_reducing(&mut images, &mut sizes, (&models, &bordered), 4096).unwrap();
        assert_eq!(find(&images, "item/sword0").1, 512);
        assert_eq!(sizes["item/sword0"], (1024, 1024));
        for (name, side) in [
            ("entity/zombie", 1024),
            ("particle/flame", 64),
            ("particle/big", 258),
        ] {
            let image = find(&images, name);
            assert_eq!((image.1, image.2), (side, side), "{name}");
            assert!(!sizes.contains_key(name), "{name}");
        }
        assert_eq!(find(&images, "particle/big"), &bordered_particle());
    }

    #[test]
    fn a_pack_too_large_even_with_its_models_reduced_halves_its_large_sprites() {
        // A 128x pack's mob textures: 40 sprites of 2048x2048 fit a 16384-wide atlas, but not a
        // GPU that allows 4096, where the pack failed to load.
        let mut images: Vec<PackedImage> = (0..40)
            .map(|i| solid(&format!("entity/mob{i}"), 2048, 9))
            .collect();
        images.push(solid("item/sword", 512, 4));
        images.push(bordered_particle());
        images.push(solid("font/ascii", 128, 1));
        images.push(solid("gui/container/generic_54", 256, 2));
        images.push(solid("horizon/map0", 128, 3));
        images.push(("horizon/slot".into(), 256, 256, Vec::new()));
        images.push(solid("entity/small", 32, 5));
        let mut images = sorted(images);
        assert!(HudAtlas::layout(&images, MAX_ATLAS_SIZE).is_ok());
        let models = HashSet::from(["item/sword".to_owned()]);
        let bordered = HashSet::from(["particle/big".to_owned()]);
        let mut sizes = HashMap::new();
        let (width, _, free) =
            layout_reducing(&mut images, &mut sizes, (&models, &bordered), 4096).unwrap();
        assert!(width <= 4096 && free + GLYPH_ROWS <= 4096);
        // Halved until they fit (2048 -> 512), drawn at their size.
        let mob = find(&images, "entity/mob0");
        assert_eq!((mob.1, mob.2, mob.3.len()), (512, 512, 512 * 512 * 4));
        assert!(mob.3.iter().all(|&v| v == 9));
        assert_eq!(sizes["entity/mob0"], (2048, 2048));
        // The model sprite went down to 16 texels first.
        assert_eq!(find(&images, "item/sword").1, 16);
        assert_eq!(sizes["item/sword"], (512, 512));
        // The blurred particle: its inside halved twice (256 -> 64) at its size before the
        // border, which wraps again from the halved edges.
        let particle = find(&images, "particle/big");
        assert_eq!((particle.1, particle.2), (66, 66));
        assert_eq!(sizes["particle/big"], (256, 256));
        let inner: Vec<u8> = (0..64 * 64).flat_map(|i| [(i % 64 * 4) as u8; 4]).collect();
        assert_eq!(
            particle,
            &wrap_border(&("particle/big".into(), 64, 64, inner))
        );
        // The font, GUI sprite, run-time slots and the small sprite keep every texel.
        for (name, side) in [
            ("font/ascii", 128),
            ("gui/container/generic_54", 256),
            ("horizon/map0", 128),
            ("horizon/slot", 256),
            ("entity/small", 32),
        ] {
            let image = find(&images, name);
            assert_eq!((image.1, image.2), (side, side), "{name}");
            assert!(!sizes.contains_key(name), "{name}");
        }
    }

    #[test]
    fn a_pack_that_fits_the_gpu_keeps_every_sprite() {
        let mut images: Vec<PackedImage> =
            (0..400).map(|i| solid(&format!("i{i}"), 64, 0)).collect();
        let mut sizes = HashMap::new();
        layout_halving(&mut images, &mut sizes, &HashSet::new(), 8192).unwrap();
        assert!(images.iter().all(|i| (i.1, i.2) == (64, 64)));
        assert!(sizes.is_empty());
    }

    #[test]
    fn a_pack_of_only_kept_sprites_still_fails_when_too_large() {
        let mut images: Vec<PackedImage> = (0..40)
            .map(|i| solid(&format!("gui/big{i}"), 2048, 0))
            .collect();
        let error =
            layout_halving(&mut images, &mut HashMap::new(), &HashSet::new(), 4096).unwrap_err();
        assert!(
            error.to_string().contains("do not fit a 4096x4096 atlas"),
            "{error}"
        );
    }

    #[test]
    fn the_limit_is_the_gpus_up_to_the_largest_atlas_side() {
        note_limit(32768);
        assert_eq!(limit(), MAX_ATLAS_SIZE);
        note_limit(4096);
        assert_eq!(limit(), 4096);
        note_limit(MAX_ATLAS_SIZE);
    }
}
