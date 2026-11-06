//! RT lighting's opacity micromaps (the RTX 4090 build, `rtx` feature, `HORIZON_RT_OMM=0` turns them off;
//! horizon-rtx `micromap` and `omm`): each cutout section's micromaps, worked out on the CPU from
//! its quads' sprites and texture coordinates, for the raw build of its structure (`hw_rays`).
//! Without the feature, or the device's extension, nothing: wgpu builds every structure as before.
//!
//! They are worked out by the meshing worker that meshed the section (`for_mesh`, called after
//! `SectionMesh::sort_solid`), each worker with its own cache, and travel with the mesh: the main
//! thread only keeps them for the section's build. On the main thread a forest section took a
//! frame up to 48 ms; on threads of their own the sections waited for them and the structures'
//! builds spread over many more frames, each rebuilding the top level (about 4 ms of the
//! renderer's p50 on FIXE01). `HORIZON_RT_SKIP` bit 262144 works them out on the main thread at
//! the upload, as before, to compare.

use super::SectionKey;
use crate::mesher::{OPAQUE_LAYER, SectionMesh, TerrainVertex, UV_STEPS};

#[cfg(feature = "rtx")]
pub use on::{Micromaps, Worked, for_mesh};

#[cfg(feature = "rtx")]
mod on {
    use super::*;
    use horizon_rtx::micromap::{AlphaMask, Cache, Layer, SectionMicromaps};
    use horizon_rtx::omm::{Geometry, Held, RawSections, Section};
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, RwLock};

    /// What a sprite layer is to the alpha test.
    enum Alpha {
        Opaque,
        Changing,
        Mask(AlphaMask),
    }

    /// The sprite layers as the alpha test sees them, and the first taken as opaque.
    struct Sprites {
        layers: Vec<Alpha>,
        split: u32,
    }

    /// The sprites the workers read, with their generation: set by `set_sprites`, none without
    /// a renderer whose device builds micromaps.
    static SPRITES: RwLock<Option<(u64, Arc<Sprites>)>> = RwLock::new(None);
    static GENERATION: AtomicU64 = AtomicU64::new(0);
    /// The chunk the rays' structures are centred on (`set_center`), x in the high half: the
    /// workers leave out sections too far to get a structure.
    static CENTER: AtomicU64 = AtomicU64::new(u64::MAX);

    thread_local! {
        /// A worker's cache, and the sprites' generation it holds triangles of.
        static CACHE: RefCell<(u64, Cache)> = RefCell::new((0, Cache::default()));
    }

    /// `HORIZON_RT_SKIP` bit 262144: the micromaps worked out on the main thread at the upload.
    fn inline() -> bool {
        static INLINE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *INLINE.get_or_init(|| {
            std::env::var("HORIZON_RT_SKIP")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .is_some_and(|skip| skip & 262_144 != 0)
        })
    }

    /// A section's micromaps as a worker found them: the sprites' generation they are of, and
    /// none where no triangle helps.
    pub struct Worked {
        generation: u64,
        planned: Option<Planned>,
    }

    /// The cutout geometries of a section's solid quads: `(geometry, first quad, quads)`.
    fn cutouts(quads: u32, runs: &[(u32, u32, bool)]) -> Vec<(u32, u32, u32)> {
        super::super::hw_rays::solid_geometries(quads, runs)
            .iter()
            .enumerate()
            .filter(|(_, g)| !g.2)
            .map(|(i, &(first, count, _))| (i as u32, first, count))
            .collect()
    }

    /// A section's micromaps from its cutout `geometries` and its `solid` quads.
    fn plan(
        cache: &mut Cache,
        sprites: &Sprites,
        geometries: &[(u32, u32, u32)],
        solid: &[TerrainVertex],
    ) -> Option<SectionMicromaps> {
        let quad = |q: u32| {
            let v = &solid[q as usize * 4..q as usize * 4 + 4];
            (
                v[0].layer,
                [0, 1, 2, 3].map(|i| v[i].uv.map(|c| f32::from(c) / UV_STEPS)),
            )
        };
        let layer = |l: u16| {
            let index = u32::from(l & !OPAQUE_LAYER);
            if l & OPAQUE_LAYER != 0 || index >= sprites.split {
                return Layer::Opaque;
            }
            match sprites.layers.get(index as usize) {
                Some(Alpha::Opaque) => Layer::Opaque,
                Some(Alpha::Mask(mask)) => Layer::Mask(mask),
                Some(Alpha::Changing) | None => Layer::Changing,
            }
        };
        cache.section(geometries, quad, layer)
    }

    /// Section `key`'s micromaps, on the meshing worker that just sorted its solid quads; none
    /// without sprites, for a section out of the structures' reach, or with `inline`.
    pub fn for_mesh(key: SectionKey, mesh: &SectionMesh) -> Option<Worked> {
        if inline() {
            return None;
        }
        let center = CENTER.load(Ordering::Relaxed);
        if center != u64::MAX {
            let (cx, cz) = ((center >> 32) as u32 as i32, center as u32 as i32);
            // Two chunks of margin: the camera moves while the section is on its way.
            let reach = super::super::hw_rays::RADIUS + 2;
            if (key.0 - cx).abs() > reach || (key.2 - cz).abs() > reach {
                return None;
            }
        }
        let (generation, sprites) = SPRITES.read().ok()?.as_ref()?.clone();
        let quads = mesh.sides.quads();
        if mesh.solid.len() < quads as usize * 4 {
            return None;
        }
        let geometries = cutouts(quads, &mesh.sides.geometries());
        if geometries.is_empty() {
            return Some(Worked {
                generation,
                planned: None,
            });
        }
        let planned = CACHE.with_borrow_mut(|(held, cache)| {
            if *held != generation {
                cache.clear();
                *held = generation;
            }
            plan(cache, &sprites, &geometries, &mesh.solid)
        });
        Some(Worked {
            generation,
            planned: planned.map(Arc::new),
        })
    }

    struct Inner {
        raw: RawSections,
        /// The sprites, for `inline`, and the main thread's cache.
        sprites: Option<Arc<Sprites>>,
        generation: u64,
        cache: Cache,
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
                sprites: None,
                generation: 0,
                cache: Cache::default(),
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
            let layers: Vec<Alpha> = (0..layers)
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
            let masks = layers
                .iter()
                .filter(|l| matches!(l, Alpha::Mask(_)))
                .count();
            let changing = layers
                .iter()
                .filter(|l| matches!(l, Alpha::Changing))
                .count();
            log::info!(
                "Horizon RTX: micromaps from {} sprite layers ({size}x{size}, {} bytes of pixels): {masks} cutouts, {changing} animated or missing, the rest opaque",
                layers.len(),
                pixels.len()
            );
            let given = !layers.is_empty();
            let sprites = Arc::new(Sprites { layers, split });
            // Meshes worked out with the old sprites come with the old generation: not kept.
            let generation = GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
            inner.generation = generation;
            inner.cache.clear();
            inner.meshed.clear();
            inner.sprites = given.then(|| sprites.clone());
            if let Ok(mut shared) = SPRITES.write() {
                *shared = given.then_some((generation, sprites));
            }
        }

        /// The chunk the rays' structures are centred on: the workers work out the micromaps of
        /// the sections around it only.
        pub fn set_center(&mut self, center: (i32, i32)) {
            if self.0.is_some() {
                let packed = (u64::from(center.0 as u32) << 32) | u64::from(center.1 as u32);
                CENTER.store(packed, Ordering::Relaxed);
            }
        }

        /// Section `key` meshed again: its `quads` solid quads `solid`, in `runs`
        /// (`mesher::Sides::geometries`), and the micromaps its worker found (`worked`).
        pub fn meshed(
            &mut self,
            key: SectionKey,
            solid: &[TerrainVertex],
            quads: u32,
            runs: &[(u32, u32, bool)],
            worked: Option<&Worked>,
        ) {
            let Some(inner) = &mut self.0 else {
                return;
            };
            inner.meshed.remove(&key);
            let Some(sprites) = &inner.sprites else {
                return;
            };
            let planned = match worked {
                Some(w) if w.generation == inner.generation => {
                    if w.planned.is_none() {
                        return;
                    }
                    w.planned.clone()
                }
                // Out of the workers' reach then, or older sprites: worked out here with
                // `inline` alone, else built without (its cutouts alpha-tested by the shader).
                _ if inline() && solid.len() >= quads as usize * 4 => {
                    let geometries = cutouts(quads, runs);
                    if geometries.is_empty() {
                        return;
                    }
                    plan(&mut inner.cache, sprites, &geometries, solid).map(Arc::new)
                }
                _ => return,
            };
            inner.seen[0] += 1;
            if let Some(m) = planned {
                inner.seen[1] += 1;
                inner.meshed.insert(key, m);
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
    /// Nothing without the `rtx` feature.
    pub struct Worked;

    /// Nothing without the `rtx` feature.
    pub fn for_mesh(_: SectionKey, _: &SectionMesh) -> Option<Worked> {
        let _ = (OPAQUE_LAYER, UV_STEPS);
        None
    }

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
        pub fn set_center(&mut self, _: (i32, i32)) {}
        pub fn meshed(
            &mut self,
            _: SectionKey,
            _: &[TerrainVertex],
            _: u32,
            _: &[(u32, u32, bool)],
            _: Option<&Worked>,
        ) {
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
pub use off::{Micromaps, Worked, for_mesh};
#[cfg(feature = "rtx")]
pub use on::Hold;
