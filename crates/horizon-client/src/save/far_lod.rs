//! Saved chunks in far terrain: `Persistent`'s far sampler reads each
//! level's saved chunks (`decode_lod_surfaces`) over the generator's far
//! samples, for tiles fine enough to show them, and the generator only for
//! tiles with a chunk not saved. A chunk queued to be saved again drops its
//! decoded columns at once (`far_changes`): edits show in the next sample.
use super::*;
use far_changes::Changed;

/// A saved chunk's far columns as read: decoded from its record, or its
/// far summary (`far_summaries`), which has every tile stride but one.
#[derive(Clone)]
pub(super) enum SavedFar {
    Columns(LodSurfaces),
    Reduced(Arc<far_summaries::Reduced>),
}
impl SavedFar {
    /// The column `width` blocks wide from `x z` in the chunk
    /// (`Column::reduced`); None when only the decoded chunk has it.
    fn reduced(&self, x: usize, z: usize, width: usize) -> Option<crate::lod::Column> {
        match self {
            Self::Columns(columns) => Some(crate::lod::Column::reduced(columns, x, z, width)),
            Self::Reduced(reduced) => reduced.column(x, z, width).cloned(),
        }
    }
    fn columns(&self) -> Option<&LodSurfaces> {
        match self {
            Self::Columns(columns) => Some(columns),
            Self::Reduced(_) => None,
        }
    }
}

#[derive(Default)]
pub(super) struct LodDecoded {
    chunks: crate::fast_hash::FxHashMap<(i32, i32), (std::time::Instant, SavedFar)>,
    order: std::collections::VecDeque<(i32, i32)>,
}
/// Saved chunks kept read for far terrain (each up to tens of kilobytes),
/// and for how long: edits saved later show after it.
const LOD_DECODED: usize = 1024;
const LOD_DECODED_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// The coarsest far tile spacing whose columns read saved chunks: 16 blocks
/// a column, and 4 with Reduced far detail (`LodSampler::set_approximate`).
pub(super) const SAVED_LOD_STRIDE: i32 = 16;
const SAVED_LOD_STRIDE_REDUCED: i32 = 4;
pub(super) struct SavedLodSampler<G: ChunkGenerator> {
    pub(super) generator: Arc<Persistent<G>>,
    pub(super) inner: Box<dyn crate::streaming::LodSampler>,
    pub(super) cache: std::collections::VecDeque<((i32, i32), Option<SavedFar>)>,
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
    /// Forget the decoded chunks saved again since the last call, and read
    /// again the details waiting for `drain_details` that were read before
    /// such a save.
    fn catch_up(&mut self) {
        let changed = self.generator.save.far_changes().since(&mut self.seen);
        let touched = |key: &(i32, i32)| match &changed {
            Changed::Chunks(chunks) => chunks.contains(key),
            Changed::All => true,
        };
        if matches!(&changed, Changed::Chunks(chunks) if chunks.is_empty()) {
            return;
        }
        self.cache.retain(|(key, _)| !touched(key));
        if !self.details.iter().any(|(key, _)| touched(key)) {
            return;
        }
        let details = std::mem::take(&mut self.details);
        for (key, columns) in details {
            let columns = if touched(&key) {
                // Read again as `request_detail` reads it; one neither saved
                // nor generated in an earlier session is left out, and far
                // terrain asks for it again.
                match self
                    .saved_columns(key)
                    .or_else(|| self.generated.get(key.0, key.1))
                {
                    Some(columns) => columns,
                    None => continue,
                }
            } else {
                columns
            };
            self.details.push((key, columns));
        }
    }
    /// Chunk `key`'s saved far columns, decoded (`full`) or as its summary
    /// has them where it has one; None where it isn't saved.
    fn saved(&mut self, key: (i32, i32), full: bool) -> Option<SavedFar> {
        let index = self.cache.iter().position(|(at, _)| *at == key);
        let cached = match index.map(|index| self.cache.remove(index).unwrap()) {
            Some((_, Some(SavedFar::Reduced(_)))) | None if full => (
                key,
                self.generator.save.lod_saved_shared(
                    key.0,
                    key.1,
                    true,
                    &self.generator.registry,
                    &self.generator.biomes,
                ),
            ),
            Some(cached) => cached,
            None => (
                key,
                self.generator.save.lod_saved_shared(
                    key.0,
                    key.1,
                    false,
                    &self.generator.registry,
                    &self.generator.biomes,
                ),
            ),
        };
        let saved = cached.1.clone();
        self.cache.push_front(cached);
        self.cache.truncate(128);
        saved
    }
    fn saved_columns(&mut self, key: (i32, i32)) -> Option<LodSurfaces> {
        self.saved(key, true)?.columns().cloned()
    }
    /// The saved column `width` blocks wide from block `x z`, None where its
    /// chunk isn't saved.
    fn saved_column(&mut self, x: i32, z: i32, width: i32) -> Option<crate::lod::Column> {
        let key = (x >> 4, z >> 4);
        let (cx, cz, width) = ((x & 15) as usize, (z & 15) as usize, width as usize);
        // One block a column: only the chunk itself has it.
        if width > 1
            && let Some(column) = self.saved(key, false)?.reduced(cx, cz, width)
        {
            return Some(column);
        }
        let columns = self.saved_columns(key)?;
        Some(crate::lod::Column::reduced(&columns, cx, cz, width))
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
        // A chunk saved since it was asked for: read again (here, and the
        // details waiting in `catch_up`), not taken from what was read when
        // it was asked for.
        self.catch_up();
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
                    self.saved_column(wx, wz, stride)
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
            && let Some(column) = self.saved_column(x, z, width)
        {
            return column;
        }
        self.inner.reduced_column(x, z, width)
    }
}

/// A saved chunk whose record couldn't be read or decoded.
struct Unread;

impl SaveDir {
    /// The chunks queued to this level's region files (`queue`).
    pub fn far_changes(&self) -> &far_changes::FarChanges {
        &self.outstanding.far
    }
    /// `chunks` were queued to be saved, readable from the pending writes:
    /// logged and their decoded columns dropped under the decoded columns'
    /// lock (taken before the log's, as `lod_saved_shared` takes them), so
    /// a far read that sees the log can't find the old columns still kept,
    /// and one that read before the log doesn't keep what it read.
    pub(super) fn far_saved(&self, chunks: impl Iterator<Item = (i32, i32)> + Clone) {
        let mut decoded = self.lod_decoded.lock().unwrap();
        self.outstanding.far.push(chunks.clone());
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
    /// `lod_saved`, read once for all far sampling threads: each saved
    /// chunk lies under far tiles of several levels, sampled on any thread.
    pub(super) fn lod_saved_shared(
        &self,
        x: i32,
        z: i32,
        full: bool,
        assets: &crate::assets::Assets,
        biomes: &Biomes,
    ) -> Option<SavedFar> {
        if let Some((at, columns)) = self.lod_decoded.lock().unwrap().chunks.get(&(x, z))
            && at.elapsed() < LOD_DECODED_TTL
            && (!full || matches!(columns, SavedFar::Columns(_)))
        {
            return Some(columns.clone());
        }
        let before = self.far_changes().end();
        let columns = self.lod_saved(x, z, full, assets, biomes)?;
        let mut decoded = self.lod_decoded.lock().unwrap();
        // Queued to be saved again while it was read: what was read may be
        // the old save, not kept (`far_saved` logs the change under this
        // lock).
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
    /// Read far summaries of this level's chunks, and write them where they
    /// are missing (`far_summaries`): far terrain reads this level.
    pub(super) fn keep_far_summaries(&self, assets: &crate::assets::Assets, biomes: &Biomes) {
        (self.outstanding.summaries)
            .get_or_init(|| Arc::new(far_summaries::FarSummaries::new(&self.root, assets, biomes)));
    }
    /// A saved chunk's far columns: unless `full` is asked for, its summary
    /// where one was written from its record as saved now; else decoded from
    /// the record (and the summary written where it was missing, unless the
    /// chunk was saved again meanwhile).
    pub(super) fn lod_saved(
        &self,
        x: i32,
        z: i32,
        full: bool,
        assets: &crate::assets::Assets,
        biomes: &Biomes,
    ) -> Option<SavedFar> {
        // A save queues the chunk (`pending`), then logs it: a save that
        // raced this read is in the log since `before`, or still pending.
        let before = self.far_changes().end();
        let pending = (self.pending.lock().ok()?).contains_key(&(Folder::Region, x, z));
        let summaries = self
            .outstanding
            .summaries
            .get()
            .filter(|_| !pending && !full);
        // No timestamp while the region is being written: decoded below.
        if let Some(summaries) = summaries
            && let Some(timestamp) = self.far_timestamp(x, z)
        {
            if let far_summaries::Summary::Columns(reduced) = summaries.read(x, z, timestamp) {
                return reduced.map(SavedFar::Reduced);
            }
            let read = self.far_columns(x, z, assets, biomes);
            // A record that couldn't be read or decoded writes no summary:
            // the next read tries again.
            if let Ok(columns) = &read
                && self.far_timestamp(x, z) == Some(timestamp)
                && !self.far_changes().changed_since(before, (x, z))
                && summaries.settled(timestamp)
            {
                let reduced = columns.as_deref().map(far_summaries::Reduced::of);
                summaries.write(x, z, timestamp, reduced.as_ref());
            }
            return read.ok().flatten().map(SavedFar::Columns);
        }
        self.lod_surfaces(x, z, assets, biomes)
            .map(SavedFar::Columns)
    }
    pub(super) fn lod_surfaces(
        &self,
        x: i32,
        z: i32,
        assets: &crate::assets::Assets,
        biomes: &Biomes,
    ) -> Option<LodSurfaces> {
        self.far_columns(x, z, assets, biomes).ok().flatten()
    }
    /// A saved chunk's far columns, None for a proto chunk (it has none), or
    /// `Unread` where its record couldn't be read or decoded.
    fn far_columns(
        &self,
        x: i32,
        z: i32,
        assets: &crate::assets::Assets,
        biomes: &Biomes,
    ) -> std::result::Result<Option<LodSurfaces>, Unread> {
        let root = self.far_record(x, z).ok_or(Unread)?;
        // Proto saves may still be missing decoration from their neighbors.
        if text(root.tag.get("Status"))
            .is_some_and(|status| status != "minecraft:full" && status != "full")
        {
            return Ok(None);
        }
        let columns = decode_lod_surfaces(
            &root,
            assets,
            biomes,
            self.shape.column(),
            self.shape.sky_light,
        );
        columns.map(Some).ok_or(Unread)
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

    /// A generator that answers every chunk asked for in detail with
    /// columns of nothing.
    #[derive(Default)]
    struct Empty(Vec<(i32, i32)>);
    impl LodSampler for Empty {
        fn sample(&mut self, _: i32, _: i32) -> Option<crate::lod::Surface> {
            None
        }
        fn request_detail(&mut self, chunks: &[(i32, i32)], _: [i32; 2]) {
            self.0.extend_from_slice(chunks);
        }
        fn drain_details(&mut self) -> Vec<crate::streaming::LodDetail> {
            let empty: LodSurfaces = Arc::new(std::array::from_fn(|_| Default::default()));
            self.0.drain(..).map(|key| (key, empty.clone())).collect()
        }
    }

    /// The test resources, a save in a fresh folder named `name`, and a
    /// chunk at 0 0 of stone at y 60, with a tower up to y 199 at 3 5 when
    /// `tower`.
    #[allow(clippy::type_complexity)]
    fn fixture(
        name: &str,
    ) -> Option<(
        crate::assets::Assets,
        Biomes,
        PathBuf,
        Arc<SaveDir>,
        impl Fn(&crate::assets::Assets, &Biomes, bool) -> NamedTag,
    )> {
        let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES")?);
        let assets = crate::assets::Assets::load(&root).expect("assets");
        let biomes = Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
            root.clone(),
        ))
        .expect("biomes");
        let chunk = |assets: &crate::assets::Assets, biomes: &Biomes, tower: bool| {
            let stone = assets.registry.default_state("minecraft:stone").unwrap();
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
            encode(&chunk, assets.registry, biomes)
        };
        let dir = std::env::temp_dir().join(format!("horizon-far-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let save = Arc::new(SaveDir::new(dir.clone(), LevelShape::OVERWORLD));
        Some((assets, biomes, dir, save, chunk))
    }

    fn sampler(
        assets: crate::assets::Assets,
        biomes: Biomes,
        dir: &Path,
        save: &Arc<SaveDir>,
        inner: Box<dyn LodSampler>,
    ) -> SavedLodSampler<crate::testgen::TestTerrain> {
        let terrain = Arc::new(crate::testgen::TestTerrain::new(assets.registry, &biomes).unwrap());
        SavedLodSampler {
            seen: save.far_changes().end(),
            generator: Arc::new(Persistent {
                inner: terrain,
                save: save.clone(),
                registry: Arc::new(assets),
                biomes: Arc::new(biomes),
            }),
            inner,
            cache: Default::default(),
            details: Vec::new(),
            approximate: false,
            generated: Arc::new(crate::lod::cache::DiskCache::new(dir.join("far"), 1)),
            bake: None,
        }
    }

    #[test]
    fn a_chunk_saved_while_it_is_generated_in_detail_answers_as_saved() {
        let Some((assets, biomes, dir, save, chunk)) = fixture("detail") else {
            return;
        };
        let tower = chunk(&assets, &biomes, true);
        let mut sampler = sampler(assets, biomes, &dir, &save, Box::<Empty>::default());
        // Not saved when asked for: the generator makes it.
        sampler.request_detail(&[(0, 0)], [0, 0]);
        save.write_records("region", vec![((0, 0), Some(tower))], false)
            .unwrap();
        let details = sampler.drain_details();
        assert_eq!(details.len(), 1);
        let top = details[0].1[5 * 16 + 3].surface.map(|s| s.height);
        assert_eq!(top, Some(200));
        save.flush().unwrap();
        drop(sampler);
        drop(save);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_detail_waiting_while_its_chunk_is_saved_again_is_read_again() {
        let Some((assets, biomes, dir, save, chunk)) = fixture("detail-again") else {
            return;
        };
        let (flat, tower) = (
            chunk(&assets, &biomes, false),
            chunk(&assets, &biomes, true),
        );
        save.write_records("region", vec![((0, 0), Some(flat))], true)
            .unwrap();
        let mut sampler = sampler(assets, biomes, &dir, &save, Box::new(Unasked));
        // Saved when asked for: its detail waits, read from the save.
        sampler.request_detail(&[(0, 0)], [0, 0]);
        save.write_records("region", vec![((0, 0), Some(tower))], false)
            .unwrap();
        let details = sampler.drain_details();
        assert_eq!(details.len(), 1);
        let top = details[0].1[5 * 16 + 3].surface.map(|s| s.height);
        assert_eq!(top, Some(200));
        save.flush().unwrap();
        drop(sampler);
        drop(save);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_saved_tile_skips_the_generator_and_shows_a_build_once_saved() {
        let Some((assets, biomes, dir, save, chunk)) = fixture("lod") else {
            return;
        };
        let (flat, tower) = (
            chunk(&assets, &biomes, false),
            chunk(&assets, &biomes, true),
        );
        save.write_records("region", vec![((0, 0), Some(flat))], true)
            .unwrap();
        let mut sampler = sampler(assets, biomes, &dir, &save, Box::new(Unasked));
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
