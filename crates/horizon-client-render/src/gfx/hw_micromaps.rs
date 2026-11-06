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
    use std::sync::Mutex;
    use std::sync::mpsc::{Receiver, Sender, channel};

    /// The frames a section's structure waits for its micromaps at most: past them (the thread
    /// behind by that much, or gone) it is built without, its cutouts alpha-tested by the shader.
    const MOST_FRAMES: u32 = 90;

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

    /// What the micromap thread is asked.
    enum Ask {
        /// New sprites: the cache starts again.
        Sprites(Arc<Sprites>),
        /// A section's alpha-tested geometries `(geometry, first quad, quads)` and every solid
        /// quad's sprite layer and texture coordinates (`UV_STEPS` units), under job number `job`.
        Section {
            key: SectionKey,
            job: u64,
            geometries: Vec<(u32, u32, u32)>,
            quads: Vec<(u16, [[u16; 2]; 4])>,
        },
    }

    /// A section's micromaps worked out, or none (no triangle helps).
    type Done = (SectionKey, u64, Option<SectionMicromaps>);

    /// The latest job asked for each waiting section, shared with the threads: one remeshed
    /// since is skipped, not worked out for nothing.
    type Latest = Arc<Mutex<HashMap<SectionKey, u64>>>;

    /// `HORIZON_RT_SKIP` bit 262144: the micromaps worked out on the main thread as the section
    /// is uploaded, as before the micromap threads (to compare).
    fn inline() -> bool {
        static INLINE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *INLINE.get_or_init(|| {
            std::env::var("HORIZON_RT_SKIP")
                .ok()
                .and_then(|v| v.parse::<u32>().ok())
                .is_some_and(|skip| skip & 262_144 != 0)
        })
    }

    /// A section's micromaps from its cutout `geometries` and its `quads`.
    fn plan(
        cache: &mut Cache,
        sprites: &Sprites,
        geometries: &[(u32, u32, u32)],
        quads: &[(u16, [[u16; 2]; 4])],
    ) -> Option<SectionMicromaps> {
        let quad = |q: u32| {
            let (layer, uv) = quads[q as usize];
            (layer, uv.map(|c| c.map(|c| f32::from(c) / UV_STEPS)))
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

    /// A micromap thread: its cache and the sprites, off the main thread. A section in a
    /// forest took the frame up to 48 ms there when it was meshed.
    fn work(asks: Receiver<Ask>, done: Sender<Done>, latest: Latest) {
        let mut cache = Cache::default();
        let mut sprites: Option<Arc<Sprites>> = None;
        while let Ok(ask) = asks.recv() {
            match ask {
                Ask::Sprites(new) => {
                    cache.clear();
                    sprites = Some(new);
                }
                Ask::Section {
                    key,
                    job,
                    geometries,
                    quads,
                } => {
                    let Some(sprites) = &sprites else {
                        continue;
                    };
                    let current = latest
                        .lock()
                        .is_ok_and(|latest| latest.get(&key) == Some(&job));
                    if !current {
                        continue;
                    }
                    let planned = plan(&mut cache, sprites, &geometries, &quads);
                    if done.send((key, job, planned)).is_err() {
                        return;
                    }
                }
            }
        }
    }

    struct Inner {
        raw: RawSections,
        /// To the micromap threads (`work`), and back.
        asks: Vec<Sender<Ask>>,
        latest: Latest,
        done: Receiver<Done>,
        /// The sprites given (no section is asked before), and the main thread's cache for
        /// them (`inline`).
        sprites: Option<Arc<Sprites>>,
        cache: Cache,
        /// The sections asked of the thread and not back yet: their latest job's number and the
        /// frames since (past `MOST_FRAMES` the structure is built without them).
        waiting: HashMap<SectionKey, (u64, u32)>,
        next_job: u64,
        /// The sections meshed since their structure was built, and their micromaps.
        meshed: HashMap<SectionKey, Arc<SectionMicromaps>>,
        /// Sections with alpha-tested geometries meshed, and those given micromaps (for the log).
        seen: [u64; 2],
    }

    impl Inner {
        /// What the thread finished: kept for the latest job of each section only.
        fn collect(&mut self) {
            while let Ok((key, job, planned)) = self.done.try_recv() {
                if self.waiting.get(&key).map(|w| w.0) != Some(job) {
                    continue;
                }
                self.waiting.remove(&key);
                if let Ok(mut latest) = self.latest.lock() {
                    latest.remove(&key);
                }
                if let Some(m) = planned {
                    self.seen[1] += 1;
                    self.meshed.insert(key, Arc::new(m));
                }
            }
        }
    }

    pub struct Micromaps(Option<Inner>);

    /// A built section's micromap, held as long as its structure.
    pub type Hold = Held;
    /// A section's micromaps, ready for its build.
    pub type Planned = Arc<SectionMicromaps>;

    impl Micromaps {
        pub fn new(device: &wgpu::Device) -> Self {
            Self(RawSections::new(device).and_then(|raw| {
                let (finished, done) = channel();
                let latest = Latest::default();
                // A quarter of the cores, 2 to 4 threads: one fell behind in fast flight, its
                // sections built without micromaps after their wait.
                let threads = std::thread::available_parallelism()
                    .map_or(2, |n| n.get() / 4)
                    .clamp(2, 4);
                let asks: Vec<_> = (0..threads)
                    .filter_map(|i| {
                        let (asks, asked) = channel();
                        let (finished, latest) = (finished.clone(), latest.clone());
                        std::thread::Builder::new()
                            .name(format!("Horizon micromaps {i}"))
                            .spawn(move || work(asked, finished, latest))
                            .map_err(|e| log::warn!("Horizon RTX: no micromap thread: {e}"))
                            .ok()
                            .map(|_| asks)
                    })
                    .collect();
                if asks.is_empty() {
                    return None;
                }
                Some(Inner {
                    raw,
                    asks,
                    latest,
                    done,
                    sprites: None,
                    cache: Cache::default(),
                    waiting: HashMap::new(),
                    next_job: 0,
                    meshed: HashMap::new(),
                    seen: [0; 2],
                })
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
            // Results of the old sprites' jobs come back for no section waiting.
            inner.waiting.clear();
            inner.meshed.clear();
            if let Ok(mut latest) = inner.latest.lock() {
                latest.clear();
            }
            let sprites = Arc::new(Sprites { layers, split });
            inner.cache.clear();
            inner.sprites = given.then(|| sprites.clone());
            for asks in &inner.asks {
                let _ = asks.send(Ask::Sprites(sprites.clone()));
            }
        }

        /// Section `key` meshed again: its `quads` solid quads `solid`, in `runs`
        /// (`mesher::Sides::geometries`). Its micromaps are worked out on the micromap thread;
        /// the section waits for them (`waiting`) before its structure is built.
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
            if inner.waiting.remove(&key).is_some()
                && let Ok(mut latest) = inner.latest.lock()
            {
                latest.remove(&key);
            }
            if inner.sprites.is_none() || solid.len() < quads as usize * 4 {
                return;
            }
            let geometries: Vec<_> = super::super::hw_rays::solid_geometries(quads, runs)
                .iter()
                .enumerate()
                .filter(|(_, g)| !g.2)
                .map(|(i, &(first, count, _))| (i as u32, first, count))
                .collect();
            if geometries.is_empty() {
                return;
            }
            // Only the quads the cutout geometries hold are read; the others stay zero.
            let mut copied = vec![(0u16, [[0u16; 2]; 4]); quads as usize];
            for &(_, first, count) in &geometries {
                for q in first..first + count {
                    let v = &solid[q as usize * 4..q as usize * 4 + 4];
                    copied[q as usize] = (v[0].layer, [0, 1, 2, 3].map(|i| v[i].uv));
                }
            }
            inner.seen[0] += 1;
            if inline()
                && let Some(sprites) = &inner.sprites
            {
                if let Some(m) = plan(&mut inner.cache, sprites, &geometries, &copied) {
                    inner.seen[1] += 1;
                    inner.meshed.insert(key, Arc::new(m));
                }
                return;
            }
            let job = inner.next_job;
            inner.next_job += 1;
            if let Ok(mut latest) = inner.latest.lock() {
                latest.insert(key, job);
            }
            let thread = (job % inner.asks.len() as u64) as usize;
            if inner.asks[thread]
                .send(Ask::Section {
                    key,
                    job,
                    geometries,
                    quads: copied,
                })
                .is_ok()
            {
                inner.waiting.insert(key, (job, 0));
            }
            if inner.seen[0] % 500 == 1 {
                log::info!(
                    "Horizon RTX: {} sections with alpha-tested quads meshed, {} given micromaps",
                    inner.seen[0],
                    inner.seen[1]
                );
            }
        }

        /// Whether section `key`'s micromaps are still being worked out: its structure waits.
        pub fn waiting(&mut self, key: SectionKey) -> bool {
            let Some(inner) = &mut self.0 else {
                return false;
            };
            if inner.waiting.is_empty() {
                return false;
            }
            inner.collect();
            inner
                .waiting
                .get(&key)
                .is_some_and(|&(_, frames)| frames < MOST_FRAMES)
        }

        /// Forget the sections `keep` turns down (out of the rays' range).
        pub fn retain(&mut self, keep: impl Fn(SectionKey) -> bool) {
            if let Some(inner) = &mut self.0 {
                inner.meshed.retain(|k, _| keep(*k));
                if inner.waiting.iter().any(|(k, _)| !keep(*k)) {
                    inner.waiting.retain(|k, _| keep(*k));
                    if let Ok(mut latest) = inner.latest.lock() {
                        latest.retain(|k, _| keep(*k));
                    }
                }
            }
        }

        /// Section `key`'s micromaps, once.
        pub fn take(&mut self, key: SectionKey) -> Option<Planned> {
            let inner = self.0.as_mut()?;
            inner.collect();
            inner.meshed.remove(&key)
        }

        /// The next frame (frees what the GPU is done with, keeps what the thread finished).
        pub fn frame(&mut self) {
            if let Some(inner) = &mut self.0 {
                inner.collect();
                for (_, frames) in inner.waiting.values_mut() {
                    *frames += 1;
                }
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
        pub fn waiting(&mut self, _: SectionKey) -> bool {
            false
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
