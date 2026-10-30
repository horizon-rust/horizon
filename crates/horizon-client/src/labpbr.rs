//! LabPBR 1.3 materials (shaderlabs.org's LabPBR Material Standard): a resource pack's
//! `<texture>_n.png` and `<texture>_s.png` beside a block texture, and the optional
//! `<texture>_sss.png` of NVIDIA's RTX Remix materials pack. `sprites.rs` loads them into
//! layers aligned with the block sprites (same layer, size, mip levels and animation frames),
//! the renderer keeps them in texture arrays parallel to the sprite arrays (`gfx/materials.rs`),
//! and [`decode`] is what a texel means.
//!
//! - `_n`: red and green the normal's X (right) and Y (down, DirectX), Z rebuilt; blue the
//!   material's ambient occlusion (255 none); alpha the height (255 the surface, 0 a quarter of
//!   a block deep).
//! - `_s`: red the perceptual smoothness; green F0 (0 to 229, linear), a predefined metal (230
//!   to 237) or a metal whose F0 is the albedo (238 to 255); blue porosity (0 to 64) or
//!   subsurface scattering (65 to 255); alpha emission (0 to 254; 255, an image without alpha,
//!   none).
//! - `_sss` (not LabPBR; RTX Remix for Minecraft's own addition): the colour of the light that
//!   comes through the material, RGB, for the subsurface scattering `_s` blue gives the
//!   intensity of. It replaces the albedo in the transmitted light: a leaf whose albedo is a dull
//!   grey-green lets a bright yellow-green through.
//!
//! A sprite without a map gets [`NEUTRAL`]: a flat, unoccluded, rough dielectric surface that
//! emits nothing.

use std::path::Path;

/// The maps, in the order [`NEUTRAL`] and `MaterialPixels::maps` keep them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Map {
    Normal,
    Specular,
    /// The backlight tint (`_sss`).
    Subsurface,
}
impl Map {
    pub const ALL: [Self; 3] = [Self::Normal, Self::Specular, Self::Subsurface];
    /// How many kinds of map there are.
    pub const COUNT: usize = Self::ALL.len();
    /// The file name's suffix before `.png`.
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Normal => "_n",
            Self::Specular => "_s",
            Self::Subsurface => "_sss",
        }
    }
}

/// The texel of a sprite without that map: a flat normal at the surface with no occlusion,
/// and a fully rough surface of F0 0 (the shader's default reflectance), neither porous nor
/// translucent, that emits nothing. Without an `_sss` map the light through the material takes
/// the albedo's colour, so its texel is only a placeholder.
pub const NEUTRAL: [[u8; 4]; Map::COUNT] =
    [[128, 128, 255, 255], [0, 0, 0, 255], [255, 255, 255, 255]];

/// A sprite layer without that map in [`MapSet::index`].
pub const NO_MAP: u32 = u32::MAX;

/// The maps of one size: square layers of `size` texels, each the map of one sprite layer.
#[derive(Clone, Debug, Default)]
pub struct MapClass {
    pub size: u32,
    /// The sprite layer each of its layers belongs to.
    pub layers: Vec<u32>,
    /// Each layer's first frame, layer major, `size` by `size` RGBA.
    pub pixels: Vec<u8>,
    /// Each layer's mip settings ([`crate::mipmap::Strategy::Data`]).
    pub mips: Vec<crate::mipmap::Settings>,
}

/// One kind of map ([`Map`]) of every sprite layer, each at its own size: sprites without
/// the map take no room, and a 16x map stays 16x when the sprite arrays are larger.
#[derive(Clone, Debug, Default)]
pub struct MapSet {
    /// By sprite layer: `class << 16 | layer` in [`MapSet::classes`], or [`NO_MAP`].
    pub index: Vec<u32>,
    /// By size, smallest first.
    pub classes: Vec<MapClass>,
}
impl MapSet {
    /// Where sprite `layer`'s map is: its class and its layer there.
    pub fn slot(&self, layer: u32) -> Option<(usize, u32)> {
        let entry = *self.index.get(layer as usize)?;
        (entry != NO_MAP).then_some(((entry >> 16) as usize, entry & 0xffff))
    }
    /// How many sprite layers have the map.
    pub fn count(&self) -> usize {
        self.classes.iter().map(|c| c.layers.len()).sum()
    }
    /// Bytes of their first mip level.
    pub fn bytes(&self) -> usize {
        self.classes.iter().map(|c| c.pixels.len()).sum()
    }
}

/// Every sprite layer's maps, made by `Sprites::finish` when a resource pack has at least one.
#[derive(Clone, Debug, Default)]
pub struct MaterialPixels {
    /// By [`Map`]: the normal, specular and subsurface maps.
    pub maps: [MapSet; Map::COUNT],
}
impl MaterialPixels {
    /// Whether sprite `layer` has `map`.
    pub fn has(&self, layer: u32, map: Map) -> bool {
        self.maps[map as usize].slot(layer).is_some()
    }
}

/// The material a resource pack declares (`optifine/texture.properties`'s `format`, as Iris
/// reads it): `lab-pbr/1.3` for LabPBR packs such as SPBR. Maps load whatever it says; this
/// only tells which standard they follow.
pub fn declared_format(root: &Path) -> Option<String> {
    let text =
        std::fs::read_to_string(root.join("assets/minecraft/optifine/texture.properties")).ok()?;
    text.lines()
        .filter_map(|line| line.split('#').next())
        .filter_map(|line| line.split_once('='))
        .find(|(key, _)| key.trim() == "format")
        .map(|(_, value)| value.trim().to_owned())
}

/// LabPBR's predefined metals (green 230 to 237), with their refractive index `n` and
/// extinction `k` per colour channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metal {
    Iron,
    Gold,
    Aluminum,
    Chrome,
    Copper,
    Lead,
    Platinum,
    Silver,
    /// 238 to 255: a metal whose F0 is the albedo.
    Albedo,
}
impl Metal {
    /// `[n, k]`, each red, green, blue.
    pub fn nk(self) -> Option<[[f32; 3]; 2]> {
        Some(match self {
            Self::Iron => [[2.9114, 2.9497, 2.5845], [3.0893, 2.9318, 2.7670]],
            Self::Gold => [[0.18299, 0.42108, 1.3734], [3.4242, 2.3459, 1.7704]],
            Self::Aluminum => [[1.3456, 0.96521, 0.61722], [7.4746, 6.3995, 5.3031]],
            Self::Chrome => [[3.1071, 3.1812, 2.3230], [3.3314, 3.3291, 3.1350]],
            Self::Copper => [[0.27105, 0.67693, 1.3164], [3.6092, 2.6248, 2.2921]],
            Self::Lead => [[1.9100, 1.8300, 1.4400], [3.5100, 3.4000, 3.1800]],
            Self::Platinum => [[2.3757, 2.0847, 1.8453], [4.2655, 3.7153, 3.1365]],
            Self::Silver => [[0.15943, 0.14512, 0.13547], [3.9291, 3.1900, 2.3808]],
            Self::Albedo => return None,
        })
    }
    /// The reflectance at normal incidence, `((n - 1)² + k²) / ((n + 1)² + k²)`; the albedo's
    /// for [`Metal::Albedo`].
    pub fn f0(self, albedo: [f32; 3]) -> [f32; 3] {
        let Some([n, k]) = self.nk() else {
            return albedo;
        };
        std::array::from_fn(|c| {
            let k2 = k[c] * k[c];
            ((n[c] - 1.0).powi(2) + k2) / ((n[c] + 1.0).powi(2) + k2)
        })
    }
}

/// What one texel of the two maps describes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// Unit tangent-space normal: x right, y down the texture (DirectX), z out of the surface.
    pub normal: [f32; 3],
    /// Material ambient occlusion, 1 for none.
    pub ambient_occlusion: f32,
    /// Height below the surface, 0 at the surface and 1 at a quarter block deep.
    pub depth: f32,
    /// Perceptual smoothness, 0..1.
    pub smoothness: f32,
    /// `(1 - smoothness)²`.
    pub roughness: f32,
    /// A dielectric's reflectance (0 to 229 / 255), when it is no metal.
    pub f0: f32,
    pub metal: Option<Metal>,
    /// 0..1 (blue 0 to 64).
    pub porosity: f32,
    /// 0..1 (blue 65 to 255).
    pub subsurface: f32,
    /// 0..1 (alpha 0 to 254; 255 is none).
    pub emission: f32,
}

/// The material of a normal texel and a specular texel (either may be [`NEUTRAL`]).
pub fn decode(normal: [u8; 4], specular: [u8; 4]) -> Material {
    let unit = |v: u8| f32::from(v) / 255.0;
    let x = unit(normal[0]) * 2.0 - 1.0;
    let y = unit(normal[1]) * 2.0 - 1.0;
    let z = (1.0 - x * x - y * y).max(0.0).sqrt();
    let length = (x * x + y * y + z * z).sqrt().max(1e-6);
    let smoothness = unit(specular[0]);
    let (porosity, subsurface) = match specular[2] {
        b @ 0..=64 => (f32::from(b) / 64.0, 0.0),
        b => (0.0, f32::from(b - 65) / 190.0),
    };
    let metal = match specular[1] {
        0..=229 => None,
        230 => Some(Metal::Iron),
        231 => Some(Metal::Gold),
        232 => Some(Metal::Aluminum),
        233 => Some(Metal::Chrome),
        234 => Some(Metal::Copper),
        235 => Some(Metal::Lead),
        236 => Some(Metal::Platinum),
        237 => Some(Metal::Silver),
        _ => Some(Metal::Albedo),
    };
    Material {
        normal: [x / length, y / length, z / length],
        ambient_occlusion: unit(normal[2]),
        depth: 1.0 - unit(normal[3]),
        smoothness,
        roughness: (1.0 - smoothness).powi(2),
        f0: if metal.is_some() {
            0.0
        } else {
            unit(specular[1])
        },
        metal,
        porosity,
        subsurface,
        emission: if specular[3] == 255 {
            0.0
        } else {
            f32::from(specular[3]) / 254.0
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_texels_are_a_flat_rough_dielectric() {
        let m = decode(NEUTRAL[0], NEUTRAL[1]);
        assert!(m.normal[2] > 0.999, "{:?}", m.normal);
        assert_eq!((m.ambient_occlusion, m.depth), (1.0, 0.0));
        assert_eq!(
            (m.smoothness, m.roughness, m.f0, m.metal),
            (0.0, 1.0, 0.0, None)
        );
        assert_eq!((m.porosity, m.subsurface, m.emission), (0.0, 0.0, 0.0));
    }

    #[test]
    fn channels_follow_the_standard() {
        // A normal tilted right and up, half occluded, deepest; a smooth gold that emits.
        let m = decode([255, 0, 128, 0], [255, 231, 30, 127]);
        assert!(m.normal[0] > 0.7 && m.normal[1] < -0.7 && m.normal[2] >= 0.0);
        assert!((m.ambient_occlusion - 0.502).abs() < 0.01);
        assert_eq!(m.depth, 1.0);
        assert_eq!((m.smoothness, m.roughness), (1.0, 0.0));
        assert_eq!(m.metal, Some(Metal::Gold));
        assert!((m.porosity - 30.0 / 64.0).abs() < 1e-6);
        assert_eq!(m.subsurface, 0.0);
        assert!((m.emission - 0.5).abs() < 1e-6);
        // F0 is linear (1.3): 229 is about 90 %.
        let dielectric = decode(NEUTRAL[0], [0, 229, 255, 254]);
        assert!((dielectric.f0 - 0.898).abs() < 0.001);
        assert_eq!((dielectric.subsurface, dielectric.emission), (1.0, 1.0));
        assert_eq!(decode(NEUTRAL[0], [0, 0, 65, 0]).subsurface, 0.0);
        assert_eq!(
            decode(NEUTRAL[0], [0, 255, 0, 0]).metal,
            Some(Metal::Albedo)
        );
    }

    #[test]
    fn the_maps_are_named_by_their_suffix_and_ordered_as_their_texels() {
        assert_eq!(
            Map::ALL.map(Map::suffix),
            ["_n", "_s", "_sss"],
            "files beside the texture"
        );
        assert!(Map::ALL.iter().enumerate().all(|(i, &m)| m as usize == i));
        assert_eq!(NEUTRAL.len(), Map::COUNT);
        assert_eq!(MaterialPixels::default().maps.len(), Map::COUNT);
    }

    #[test]
    fn metals_reflect_their_colour() {
        let gold = Metal::Gold.f0([0.0; 3]);
        assert!(gold[0] > 0.9 && gold[2] < 0.4, "{gold:?}");
        let iron = Metal::Iron.f0([0.0; 3]);
        assert!(iron.iter().all(|&c| (0.45..0.6).contains(&c)), "{iron:?}");
        assert_eq!(Metal::Albedo.f0([0.25, 0.5, 0.75]), [0.25, 0.5, 0.75]);
    }

    #[test]
    fn the_declared_format_is_read_from_texture_properties() {
        let root = std::env::temp_dir().join(format!("horizon-labpbr-{}", std::process::id()));
        let dir = root.join("assets/minecraft/optifine");
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(declared_format(&root), None);
        std::fs::write(
            dir.join("texture.properties"),
            "# a pack\nformat = lab-pbr/1.3 # x\n",
        )
        .unwrap();
        assert_eq!(declared_format(&root).as_deref(), Some("lab-pbr/1.3"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
