//! Saved chunks in far terrain: `Persistent`'s far sampler reads each
//! level's saved chunks (`decode_lod_surfaces`) over the generator's far
//! samples, for tiles fine enough to show them, and the generator only for
//! tiles with a chunk not saved. A chunk queued to be saved again drops its
//! decoded columns at once (`far_changes`): edits show in the next sample.
use super::*;
use far_changes::Changed;

#[derive(Default)]
pub(super) struct LodDecoded {
    chunks: crate::fast_hash::FxHashMap<(i32, i32), (std::time::Instant, LodSurfaces)>,
    order: std::collections::VecDeque<(i32, i32)>,
}
/// Saved chunks kept decoded for far terrain (each tens of kilobytes), and
/// for how long: edits saved later show after it.
const LOD_DECODED: usize = 1024;
const LOD_DECODED_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// The coarsest far tile spacing whose columns read saved chunks: 16 blocks
/// a column, and 4 with Reduced far detail (`LodSampler::set_approximate`).
pub(super) const SAVED_LOD_STRIDE: i32 = 16;
const SAVED_LOD_STRIDE_REDUCED: i32 = 4;
pub(super) struct SavedLodSampler<G: ChunkGenerator> {
    pub(super) generator: Arc<Persistent<G>>,
    pub(super) inner: Box<dyn crate::streaming::LodSampler>,
    pub(super) cache: std::collections::VecDeque<((i32, i32), Option<LodSurfaces>)>,
    pub(super) details: Vec<crate::streaming::LodDetail>,
    /// Reduced far detail: saved chunks only in tiles up to
    /// `SAVED_LOD_STRIDE_REDUCED` blocks a column.
    pub(super) approximate: bool,
    /// Chunks the generator decorated for far terrain alone, kept from
    /// earlier sessions; saved chunks take precedence over them.
    pub(super) generated: Arc<crate::lod::cache::DiskCache>,
    /// The generator's far samples from earlier sessions (`lod::bake`).
    pub(super) bake: Option<Arc<crate::lod::bake::LodBake>>,
    /// The save's change log read up to here (`cache` is as of then).
    pub(super) seen: u64,
}
impl<G: ChunkGenerator> SavedLodSampler<G> {
    /// Forget the decoded chunks saved again since the last call.
    fn catch_up(&mut self) {
        match self.generator.save.far_changes().since(&mut self.seen) {
            Changed::Chunks(chunks) if chunks.is_empty() => {}
            Changed::Chunks(chunks) => self.cache.retain(|(key, _)| !chunks.contains(key)),
            Changed::All => self.cache.clear(),
        }
    }
    fn saved_columns(&mut self, key: (i32, i32)) -> Option<LodSurfaces> {
        let cached = match self.cache.iter().position(|(at, _)| *at == key) {
            Some(index) => self.cache.remove(index).unwrap(),
            None => (
                key,
                self.generator.save.lod_surfaces_shared(
                    key.0,
                    key.1,
                    &self.generator.registry,
                    &self.generator.biomes,
                ),
            ),
        };
        let columns = cached.1.clone();
        self.cache.push_front(cached);
        self.cache.truncate(128);
        columns
    }
}
impl<G: ChunkGenerator> crate::streaming::LodSampler for SavedLodSampler<G> {
    fn fork(&self) -> Option<Box<dyn crate::streaming::LodSampler>> {
        Some(Box::new(Self {
            generator: self.generator.clone(),
            inner: self.inner.fork()?,
            cache: Default::default(),
            details: Vec::new(),
            approximate: self.approximate,
            generated: self.generated.clone(),
            bake: self.bake.clone(),
            seen: self.generator.save.far_changes().end(),
        }))
    }
    fn set_approximate(&mut self, approximate: bool) {
        self.approximate = approximate;
        self.inner.set_approximate(approximate);
    }
    fn request_detail(&mut self, chunks: &[(i32, i32)], center: [i32; 2]) {
        self.catch_up();
        let mut missing = Vec::new();
        for &key in chunks {
            // Generated chunks from earlier sessions answer only these
            // requests: reading them for every sampled cell of a coarse tile
            // slowed sampling more than it saved.
            if let Some(columns) = self
                .saved_columns(key)
                .or_else(|| self.generated.get(key.0, key.1))
            {
                self.details.push((key, columns));
            } else {
                missing.push(key);
            }
        }
        self.inner.request_detail(&missing, center);
    }
    fn drain_details(&mut self) -> Vec<crate::streaming::LodDetail> {
        let mut details = std::mem::take(&mut self.details);
        for (key, generated) in self.inner.drain_details() {
            // A generation batch also returns neighbouring, unrequested chunks.
            // Their saved builds must take precedence too.
            let columns = self.saved_columns(key).unwrap_or_else(|| {
                self.generated.put(key.0, key.1, &generated);
                generated
            });
            details.push((key, columns));
        }
        details
    }
    fn grid(&mut self, x: i32, z: i32, stride: i32, side: usize) -> Vec<crate::lod::Column> {
        self.catch_up();
        // Saved chunks show in tiles up to 16 blocks a column. With Reduced far
        // detail only up to 4 (within about a kilometre): coarser tiles keep
        // the generator's terrain, which saved chunks were made from, since
        // decoding up to 256 saved chunks for each of them held the whole far
        // view back for minutes in large saves. Edits that far away show once
        // their tiles refine on approach.
        let limit = if self.approximate {
            SAVED_LOD_STRIDE_REDUCED
        } else {
            SAVED_LOD_STRIDE
        };
        let cell = |index: usize| {
            (
                x + (index % side) as i32 * stride,
                z + (index / side) as i32 * stride,
            )
        };
        let saved: Vec<Option<crate::lod::Column>> = if stride <= limit {
            (0..side * side)
                .map(|index| {
                    let (wx, wz) = cell(index);
                    self.saved_columns((wx >> 4, wz >> 4)).map(|saved| {
                        crate::lod::Column::reduced(
                            &saved,
                            (wx & 15) as usize,
                            (wz & 15) as usize,
                            stride as usize,
                        )
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        // Every column saved: the generator has nothing to add.
        if !saved.is_empty() && saved.iter().all(Option::is_some) {
            return saved.into_iter().flatten().collect();
        }
        let approximate = self.approximate;
        let baked = (self.bake.as_ref()).and_then(|b| b.grid(approximate, [x, z], stride, side));
        let mut columns = baked.unwrap_or_else(|| {
            let columns = self.inner.grid(x, z, stride, side);
            if let Some(bake) = &self.bake {
                bake.put_grid(approximate, [x, z], stride, side, &columns);
            }
            columns
        });
        for (column, saved) in columns.iter_mut().zip(saved) {
            if let Some(saved) = saved {
                *column = saved;
            }
        }
        columns
    }
    fn sample(&mut self, x: i32, z: i32) -> Option<crate::lod::Surface> {
        self.column(x, z).surface
    }
    fn column(&mut self, x: i32, z: i32) -> crate::lod::Column {
        self.reduced_column(x, z, 1)
    }
    fn reduced_column(&mut self, x: i32, z: i32, width: i32) -> crate::lod::Column {
        self.catch_up();
        if width <= 16
            && let Some(columns) = self.saved_columns((x >> 4, z >> 4))
        {
            return crate::lod::Column::reduced(
                &columns,
                (x & 15) as usize,
                (z & 15) as usize,
                width as usize,
            );
        }
        self.inner.reduced_column(x, z, width)
    }
}

impl SaveDir {
    /// The chunks queued to this level's region files (`queue`).
    pub fn far_changes(&self) -> &far_changes::FarChanges {
        &self.outstanding.far
    }
    /// `chunks` were queued to be saved, readable from the pending writes:
    /// logged first, then their decoded columns dropped, so a far read that
    /// raced the save either finds the log or finds them gone.
    pub(super) fn far_saved(&self, chunks: impl Iterator<Item = (i32, i32)> + Clone) {
        self.outstanding.far.push(chunks.clone());
        let mut decoded = self.lod_decoded.lock().unwrap();
        for key in chunks {
            decoded.chunks.remove(&key);
        }
    }
    /// Clear `chunks` from the region files and wait for it, as a test's
    /// stand-in for any save of them.
    #[cfg(test)]
    pub(crate) fn clear_chunks(&self, chunks: &[(i32, i32)]) -> Result<()> {
        let records = chunks.iter().map(|&key| (key, None)).collect();
        self.write_records("region", records, true)
    }
    /// `lod_surfaces`, decoded once for all far sampling threads: each saved
    /// chunk lies under far tiles of several levels, sampled on any thread.
    pub(super) fn lod_surfaces_shared(
        &self,
        x: i32,
        z: i32,
        assets: &crate::assets::Assets,
        biomes: &Biomes,
    ) -> Option<LodSurfaces> {
        if let Some((at, columns)) = self.lod_decoded.lock().unwrap().chunks.get(&(x, z))
            && at.elapsed() < LOD_DECODED_TTL
        {
            return Some(columns.clone());
        }
        let before = self.far_changes().end();
        let columns = self.lod_surfaces(x, z, assets, biomes)?;
        let mut decoded = self.lod_decoded.lock().unwrap();
        // Queued to be saved again while it was read: what was read may be
        // the old save, not kept (`far_saved` logs the change before it
        // drops the decoded columns).
        if self.far_changes().changed_since(before, (x, z)) {
            return Some(columns);
        }
        let fresh = (std::time::Instant::now(), columns.clone());
        if decoded.chunks.insert((x, z), fresh).is_none() {
            decoded.order.push_back((x, z));
            while decoded.order.len() > LOD_DECODED {
                if let Some(old) = decoded.order.pop_front() {
                    decoded.chunks.remove(&old);
                }
            }
        }
        Some(columns)
    }
    pub(super) fn lod_surfaces(
        &self,
        x: i32,
        z: i32,
        assets: &crate::assets::Assets,
        biomes: &Biomes,
    ) -> Option<LodSurfaces> {
        let root = self.far_record(x, z)?;
        // Proto saves may still be missing decoration from their neighbors.
        if text(root.tag.get("Status"))
            .is_some_and(|status| status != "minecraft:full" && status != "full")
        {
            return None;
        }
        decode_lod_surfaces(
            &root,
            assets,
            biomes,
            self.shape.column(),
            self.shape.sky_light,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::streaming::LodSampler;

    /// A generator that must not be asked: every column is saved.
    struct Unasked;
    impl LodSampler for Unasked {
        fn sample(&mut self, _: i32, _: i32) -> Option<crate::lod::Surface> {
            panic!("a tile of saved chunks needs no generator")
        }
        fn grid(&mut self, _: i32, _: i32, _: i32, _: usize) -> Vec<crate::lod::Column> {
            panic!("a tile of saved chunks needs no generator")
        }
    }

    #[test]
    fn a_saved_tile_skips_the_generator_and_shows_a_build_once_saved() {
        let Some(root) = std::env::var_os("HORIZON_TEST_RESOURCES") else {
            return;
        };
        let root = PathBuf::from(root);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
            root.clone(),
        ))
        .expect("biomes");
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let chunk = |tower: bool| {
            let mut chunk = Chunk::empty(0, 0);
            for i in 0..256i32 {
                chunk.set(i & 15, 60, i >> 4, stone.0);
            }
            if tower {
                for y in 61..200 {
                    chunk.set(3, y, 5, stone.0);
                }
            }
            chunk.lit = true;
            encode(&chunk, assets.registry, &biomes)
        };
        let dir = std::env::temp_dir().join(format!("horizon-far-lod-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (flat, tower) = (chunk(false), chunk(true));
        let save = Arc::new(SaveDir::new(dir.clone(), LevelShape::OVERWORLD));
        save.write_records("region", vec![((0, 0), Some(flat))], true)
            .unwrap();
        let inner = Arc::new(crate::testgen::TestTerrain::new(assets.registry, &biomes).unwrap());
        let mut sampler = SavedLodSampler {
            seen: save.far_changes().end(),
            generator: Arc::new(Persistent {
                inner,
                save: save.clone(),
                registry: Arc::new(assets),
                biomes: Arc::new(biomes),
            }),
            inner: Box::new(Unasked),
            cache: Default::default(),
            details: Vec::new(),
            approximate: false,
            generated: Arc::new(crate::lod::cache::DiskCache::new(dir.join("far"), 1)),
            bake: None,
        };
        let height = |columns: &[crate::lod::Column], i: usize| columns[i].surface.unwrap().height;
        let before = sampler.grid(0, 0, 1, 16);
        assert_eq!(height(&before, 5 * 16 + 3), 61);
        // Built on and saved: the next tile shows it, past the decoded
        // chunks kept by the sampler and by the save.
        save.write_records("region", vec![((0, 0), Some(tower))], false)
            .unwrap();
        let after = sampler.grid(0, 0, 1, 16);
        assert_eq!(height(&after, 5 * 16 + 3), 200);
        assert_eq!(after[0], before[0]);
        save.flush().unwrap();
        drop(sampler);
        drop(save);
        let _ = fs::remove_dir_all(&dir);
    }
}
