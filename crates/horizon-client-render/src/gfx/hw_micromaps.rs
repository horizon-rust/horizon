//! RT lighting's opacity micromaps (the RTX 4090 build, `rtx` feature, `HORIZON_RT_OMM=0` turns them off;
//! horizon-rtx `micromap` and `omm`): each cutout section's micromaps, worked out on the CPU when
//! it is meshed from its quads' sprites and texture coordinates, for the raw build of its
//! structure (`hw_rays`). Without the feature, or the device's extension, nothing: wgpu builds
//! every structure as before.

use super::SectionKey;
use crate::mesher::{OPAQUE_LAYER, TerrainVertex, UV_STEPS};

#[cfg(feature = "rtx")]
pub use on::Micromaps;

#[cfg(feature = "rtx")]
mod on {
    use super::*;
    use horizon_rtx::micromap::{AlphaMask, Cache, Layer, SectionMicromaps};
    use horizon_rtx::omm::{Geometry, Held, RawSections, Section};
    use std::collections::HashMap;
    use std::sync::Arc;

    /// What a sprite layer is to the alpha test.
    enum Alpha {
        Opaque,
        Changing,
        Mask(AlphaMask),
    }

    struct Inner {
        raw: RawSections,
        cache: Cache,
        layers: Vec<Alpha>,
        split: u32,
        /// The sections meshed since their structure was built, and their micromaps.
        meshed: HashMap<SectionKey, Arc<SectionMicromaps>>,
        /// Sections with alpha-tested geometries meshed, and those given micromaps (for the log).
        seen: [u64; 2],
    }

    pub struct Micromaps(Option<Inner>);

    /// A built section's micromap, held as long as its structure.
    pub type Hold = Held;
    /// A section's micromaps, ready for its build.
    pub type Planned = Arc<SectionMicromaps>;

    impl Micromaps {
        pub fn new(device: &wgpu::Device) -> Self {
            Self(RawSections::new(device).map(|raw| Inner {
                raw,
                cache: Cache::default(),
                layers: Vec::new(),
                split: 0,
                meshed: HashMap::new(),
                seen: [0; 2],
            }))
        }

        /// The block sprites' top level, `size` texels square, `layers` of them, those of
        /// `animated` changing, the layers from `split` on taken as opaque (the shader's
        /// `SPRITE_SPLIT`).
        pub fn set_sprites(
            &mut self,
            pixels: &[u8],
            size: u32,
            layers: u32,
            animated: impl Iterator<Item = u32>,
            split: u32,
        ) {
            let Some(inner) = &mut self.0 else {
                return;
            };
            let texels = (size * size) as usize;
            let animated: std::collections::HashSet<u32> = animated.collect();
            inner.layers = (0..layers)
                .map(|l| {
                    let Some(rgba) =
                        pixels.get(l as usize * texels * 4..(l as usize + 1) * texels * 4)
                    else {
                        return Alpha::Changing;
                    };
                    if animated.contains(&l) {
                        Alpha::Changing
                    } else if rgba.chunks_exact(4).all(|p| p[3] >= 128) {
                        Alpha::Opaque
                    } else {
                        Alpha::Mask(AlphaMask::from_rgba(size, size, rgba))
                    }
                })
                .collect();
            let masks = inner
                .layers
                .iter()
                .filter(|l| matches!(l, Alpha::Mask(_)))
                .count();
            let changing = inner
                .layers
                .iter()
                .filter(|l| matches!(l, Alpha::Changing))
                .count();
            log::info!(
                "Horizon RTX: micromaps from {layers} sprite layers ({size}x{size}, {} bytes of pixels): {masks} cutouts, {changing} animated or missing, the rest opaque",
                pixels.len()
            );
            inner.split = split;
            inner.cache.clear();
            inner.meshed.clear();
        }

        /// Section `key` meshed again: its `quads` solid quads `solid`, in `runs`
        /// (`mesher::Sides::geometries`).
        pub fn meshed(
            &mut self,
            key: SectionKey,
            solid: &[TerrainVertex],
            quads: u32,
            runs: &[(u32, u32, bool)],
        ) {
            let Some(inner) = &mut self.0 else {
                return;
            };
            inner.meshed.remove(&key);
            if inner.layers.is_empty() {
                return;
            }
            let alpha_tested: Vec<_> = super::super::hw_rays::solid_geometries(quads, runs)
                .iter()
                .enumerate()
                .filter(|(_, g)| !g.2)
                .map(|(i, &(first, count, _))| (i as u32, first, count))
                .collect();
            if alpha_tested.is_empty() {
                return;
            }
            let (layers, split) = (&inner.layers, inner.split);
            let quad = |q: u32| {
                let v = &solid[q as usize * 4..q as usize * 4 + 4];
                (
                    v[0].layer,
                    [0, 1, 2, 3].map(|i| v[i].uv.map(|c| f32::from(c) / UV_STEPS)),
                )
            };
            let layer = |l: u16| {
                let index = u32::from(l & !OPAQUE_LAYER);
                if l & OPAQUE_LAYER != 0 || index >= split {
                    return Layer::Opaque;
                }
                match layers.get(index as usize) {
                    Some(Alpha::Opaque) => Layer::Opaque,
                    Some(Alpha::Mask(mask)) => Layer::Mask(mask),
                    Some(Alpha::Changing) | None => Layer::Changing,
                }
            };
            if solid.len() < quads as usize * 4 {
                return;
            }
            let planned = inner.cache.section(&alpha_tested, quad, layer);
            inner.seen[0] += 1;
            if let Some(m) = planned {
                inner.seen[1] += 1;
                inner.meshed.insert(key, Arc::new(m));
            }
            if inner.seen[0] % 500 == 1 {
                log::info!(
                    "Horizon RTX: {} sections with alpha-tested quads meshed, {} given micromaps",
                    inner.seen[0],
                    inner.seen[1]
                );
            }
        }

        /// Forget the sections `keep` turns down (out of the rays' range).
        pub fn retain(&mut self, keep: impl Fn(SectionKey) -> bool) {
            if let Some(inner) = &mut self.0 {
                inner.meshed.retain(|k, _| keep(*k));
            }
        }

        /// Section `key`'s micromaps, once.
        pub fn take(&mut self, key: SectionKey) -> Option<Planned> {
            self.0.as_mut()?.meshed.remove(&key)
        }

        /// The next frame (frees what the GPU is done with).
        pub fn frame(&mut self) {
            if let Some(inner) = &mut self.0 {
                inner.raw.frame();
            }
        }

        /// The raw builds of `sections` (a structure, its vertices, its geometries as `(first
        /// vertex, quads, opaque, once)`, its micromaps): each one's hold where it was built,
        /// and the command buffer to submit before the top-level build.
        pub fn build(
            &mut self,
            device: &wgpu::Device,
            indices: &wgpu::Buffer,
            sections: Vec<(
                &mut wgpu::Blas,
                &wgpu::Buffer,
                Vec<(u64, u32, bool, bool)>,
                &Planned,
            )>,
        ) -> (Vec<Option<Hold>>, Option<wgpu::CommandBuffer>) {
            let Some(inner) = &mut self.0 else {
                return ((0..sections.len()).map(|_| None).collect(), None);
            };
            let geometries: Vec<Vec<Geometry>> = sections
                .iter()
                .map(|s| {
                    s.2.iter()
                        .map(|&(first_vertex, quads, opaque, once)| Geometry {
                            first_vertex,
                            quads,
                            opaque,
                            once,
                        })
                        .collect()
                })
                .collect();
            let mut list: Vec<Section<'_>> = sections
                .into_iter()
                .zip(&geometries)
                .map(|((blas, vertices, _, micromaps), geometries)| Section {
                    blas,
                    vertices,
                    geometries,
                    micromaps,
                })
                .collect();
            inner.raw.build(
                device,
                indices,
                std::mem::size_of::<TerrainVertex>() as u64,
                &mut list,
            )
        }
    }
}

#[cfg(not(feature = "rtx"))]
mod off {
    use super::*;

    /// Nothing without the `rtx` feature.
    pub struct Micromaps;
    pub type Hold = ();
    pub type Planned = ();

    impl Micromaps {
        pub fn new(_: &wgpu::Device) -> Self {
            Self
        }
        pub fn set_sprites(
            &mut self,
            _: &[u8],
            _: u32,
            _: u32,
            _: impl Iterator<Item = u32>,
            _: u32,
        ) {
        }
        pub fn meshed(
            &mut self,
            _: SectionKey,
            _: &[TerrainVertex],
            _: u32,
            _: &[(u32, u32, bool)],
        ) {
            let _ = (OPAQUE_LAYER, UV_STEPS);
        }
        pub fn retain(&mut self, _: impl Fn(SectionKey) -> bool) {}
        pub fn take(&mut self, _: SectionKey) -> Option<Planned> {
            None
        }
        pub fn frame(&mut self) {}
        #[allow(clippy::type_complexity)]
        pub fn build(
            &mut self,
            _: &wgpu::Device,
            _: &wgpu::Buffer,
            sections: Vec<(
                &mut wgpu::Blas,
                &wgpu::Buffer,
                Vec<(u64, u32, bool, bool)>,
                &Planned,
            )>,
        ) -> (Vec<Option<Hold>>, Option<wgpu::CommandBuffer>) {
            ((0..sections.len()).map(|_| None).collect(), None)
        }
    }
}

#[cfg(not(feature = "rtx"))]
pub use off::Hold;
#[cfg(not(feature = "rtx"))]
pub use off::Micromaps;
#[cfg(feature = "rtx")]
pub use on::Hold;
