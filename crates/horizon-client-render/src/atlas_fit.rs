// Adapted from Starling (https://github.com/starling-client/starling, `android` branch at
// 150a414b09a1, `fit_mobile_atlas` in `crates/horizon-client-render/src/hud.rs`), written by the
// Starling contributors. Copyright (c) 2026 Ludovic Marie and the Horizon contributors, under the
// same MIT license as Horizon (LICENSE at the repository root).

//! The shared atlas when a pack's sprites don't fit the GPU's limit (or `MAX_ATLAS_SIZE`, the
//! largest side Horizon lays out): the larger sprites are halved until the atlas fits, instead of
//! the pack failing to load.

use crate::Result;
use crate::hud::{HudAtlas, MAX_ATLAS_SIZE, PackedImage, TextureFlags};
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

/// Fit `images` (before their blur borders) to the GPU's limit, and that limit: the largest atlas
/// side `HudAtlas::pack` lays out. `leave_out` (the custom item textures) is what
/// `HudAtlas::layout_leaving_out` drops when the atlas doesn't fit, so a pack that fits without
/// them keeps every texel, as before.
pub(crate) fn fit(
    images: &mut [PackedImage],
    gui_sizes: &mut HashMap<String, (u32, u32)>,
    flags: &HashMap<String, TextureFlags>,
    leave_out: &HashSet<String>,
) -> Result<u32> {
    let limit = TEXTURE_LIMIT.load(Ordering::Relaxed).min(MAX_ATLAS_SIZE);
    fit_atlas(images, gui_sizes, flags, leave_out, limit)?;
    Ok(limit)
}

/// Halve the larger sprites until the atlas, `leave_out` aside, fits `limit` texels a side. Fonts, GUI sprites,
/// Horizon's own sprites and run-time slots, and sprites of 32x32 or less keep their size. A
/// halved sprite keeps its original size in `gui_sizes`, so it draws at the same size (and with
/// the same UVs) from fewer texels. The last layout's error when nothing is left to halve.
fn fit_atlas(
    images: &mut [PackedImage],
    gui_sizes: &mut HashMap<String, (u32, u32)>,
    flags: &HashMap<String, TextureFlags>,
    leave_out: &HashSet<String>,
    limit: u32,
) -> Result<()> {
    loop {
        // The sizes `HudAtlas::pack` lays out: a blurred sprite gets a border of one texel.
        let mut sizes: Vec<PackedImage> = images
            .iter()
            .filter(|i| !leave_out.contains(&i.0))
            .map(|(name, w, h, _)| {
                let border = if flags.get(name).is_some_and(|f| f.blur) {
                    2
                } else {
                    0
                };
                (name.clone(), w + border, h + border, Vec::new())
            })
            .collect();
        sizes.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
        let Err(error) = HudAtlas::layout(&sizes, limit) else {
            return Ok(());
        };
        let mut halved = false;
        for (name, w, h, rgba) in images.iter_mut() {
            if ["font/", "gui/", "ui/", "horizon/"]
                .iter()
                .any(|p| name.starts_with(p))
                || (*w <= 32 && *h <= 32)
            {
                continue;
            }
            gui_sizes.entry(name.clone()).or_insert((*w, *h));
            let (old_w, old_h) = (*w, *h);
            *w = (*w / 2).max(1);
            *h = (*h / 2).max(1);
            // Nearest texel.
            let mut smaller = Vec::with_capacity((*w * *h * 4) as usize);
            for y in 0..*h {
                for x in 0..*w {
                    let at = ((y * old_h / *h * old_w + x * old_w / *w) * 4) as usize;
                    smaller.extend_from_slice(&rgba[at..at + 4]);
                }
            }
            *rgba = smaller;
            halved = true;
        }
        if !halved {
            return Err(error);
        }
        log::info!("halved the larger atlas sprites to fit a {limit}x{limit} atlas");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hud::GLYPH_ROWS;

    fn solid(name: &str, side: u32, value: u8) -> PackedImage {
        (
            name.into(),
            side,
            side,
            vec![value; (side * side * 4) as usize],
        )
    }

    fn layout(images: &[PackedImage], limit: u32) -> Result<(u32, Vec<(u32, u32)>, u32)> {
        let mut sorted = images.to_vec();
        sorted.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
        HudAtlas::layout(&sorted, limit)
    }

    #[test]
    fn a_pack_too_large_for_the_gpu_halves_its_large_sprites_only() {
        // A 128x pack's mob textures: 40 sprites of 2048x2048 fit a 16384-wide atlas, but not a
        // GPU that allows 4096, where the pack failed to load.
        let mut images: Vec<PackedImage> = (0..40)
            .map(|i| solid(&format!("entity/mob{i}"), 2048, 9))
            .collect();
        images.push(solid("font/ascii", 128, 1));
        images.push(solid("gui/container/generic_54", 256, 2));
        images.push(solid("horizon/map0", 128, 3));
        images.push(solid("item/diamond", 32, 4));
        assert!(layout(&images, MAX_ATLAS_SIZE).is_ok());
        assert!(layout(&images, 4096).is_err());
        let mut sizes = HashMap::new();
        fit_atlas(
            &mut images,
            &mut sizes,
            &HashMap::new(),
            &HashSet::new(),
            4096,
        )
        .unwrap();
        // Halved until they fit (2048 -> 512), drawn at their size.
        assert_eq!((images[0].1, images[0].2), (512, 512));
        assert_eq!(images[0].3.len(), 512 * 512 * 4);
        assert!(images[0].3.iter().all(|&v| v == 9));
        assert_eq!(sizes["entity/mob0"], (2048, 2048));
        // The font, GUI sprite, run-time slot and small item keep every texel.
        for (i, side) in [(40, 128), (41, 256), (42, 128), (43, 32)] {
            assert_eq!((images[i].1, images[i].2), (side, side), "{}", images[i].0);
            assert_eq!(images[i].3.len(), (side * side * 4) as usize);
            assert!(!sizes.contains_key(&images[i].0));
        }
        let (width, _, free) = layout(&images, 4096).unwrap();
        assert!(width <= 4096 && free + GLYPH_ROWS <= 4096);
    }

    #[test]
    fn a_pack_that_fits_the_gpu_keeps_every_sprite() {
        let mut images: Vec<PackedImage> =
            (0..400).map(|i| solid(&format!("i{i}"), 64, 0)).collect();
        let mut sizes = HashMap::new();
        fit_atlas(
            &mut images,
            &mut sizes,
            &HashMap::new(),
            &HashSet::new(),
            8192,
        )
        .unwrap();
        assert!(images.iter().all(|i| (i.1, i.2) == (64, 64)));
        assert!(sizes.is_empty());
    }

    #[test]
    fn a_pack_that_fits_without_its_custom_item_textures_keeps_every_sprite() {
        // Fits 8192 once the custom item textures are left out, as it loads today:
        // nothing is halved, `layout_leaving_out` drops them instead.
        let mut images: Vec<PackedImage> = (0..3)
            .map(|i| solid(&format!("entity/mob{i}"), 2048, 0))
            .collect();
        let cit: HashSet<String> = (0..40).map(|i| format!("cit/sword{i}")).collect();
        images.extend(cit.iter().map(|name| solid(name, 1024, 0)));
        assert!(layout(&images, 8192).is_err());
        let mut sizes = HashMap::new();
        fit_atlas(&mut images, &mut sizes, &HashMap::new(), &cit, 8192).unwrap();
        assert!(images.iter().all(|i| i.1 == 2048 || i.1 == 1024));
        assert!(sizes.is_empty());
    }

    #[test]
    fn a_pack_of_only_kept_sprites_still_fails_when_too_large() {
        let mut images: Vec<PackedImage> = (0..40)
            .map(|i| solid(&format!("gui/big{i}"), 2048, 0))
            .collect();
        let error = fit_atlas(
            &mut images,
            &mut HashMap::new(),
            &HashMap::new(),
            &HashSet::new(),
            4096,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("do not fit a 4096x4096 atlas"),
            "{error}"
        );
    }

    #[test]
    fn a_pack_that_fits_the_largest_atlas_side_is_untouched() {
        let mut images = vec![solid("entity/large", 2048, 5)];
        let mut sizes = HashMap::new();
        note_limit(32768);
        assert_eq!(
            fit(&mut images, &mut sizes, &HashMap::new(), &HashSet::new()).unwrap(),
            MAX_ATLAS_SIZE
        );
        note_limit(MAX_ATLAS_SIZE);
        assert_eq!((images[0].1, sizes.len()), (2048, 0));
    }
}
