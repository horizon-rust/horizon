//! Far summaries kept beside a level's chunks: each saved chunk's far
//! columns as the tiles coarser than a block a column read them (the chunk's
//! columns reduced to 2, 4, 8 and 16 blocks wide: [`Reduced`]), in region
//! files of their own under `data/horizon_far_summaries/` in the level's
//! folder, which vanilla never reads. A far read of a saved chunk is then one
//! small record instead of the whole chunk decoded. Tiles of one block a
//! column, the nearest, decode the chunk itself.
//!
//! Far terrain writes a chunk's summary when it first reads the chunk at a
//! stride a summary has and finds none (`SaveDir::lod_saved`, on its own
//! idle threads), not the region writer: the chunks saved most often are the
//! ones beside the players, whose tiles are one block a column. A summary
//! carries, as its record's timestamp, the one of the chunk record it was
//! made from, and is used only while the level's region file has the chunk
//! under that timestamp: a chunk saved again, by this game or another
//! program, makes it stale, and the next read writes it again.
//!
//! A record holds its columns as bytes (`encode_reduced`): a palette of the
//! materials (block state, biome, light) its spans use, then each column's
//! spans as varints, each from the end of the one below. It names its game
//! version, its layout ([`FORMAT`]) and a hash of the block-state and biome
//! tables its numbers index ([`tables_hash`]): a record of another layout, or
//! of a world whose mods number its states otherwise, reads as missing and is
//! written again. A proto chunk, which far terrain doesn't read, has a record
//! that says so.
use super::*;
use crate::lod::{Column, Span};
use horizon_world::region::RegionFile;

/// The folder under the level's.
const FOLDER: &str = "data/horizon_far_summaries";
/// Region files kept open; past it all are closed.
const OPEN_REGIONS: usize = 64;
/// The layout of the records, under [`FORMAT_KEY`]. 1 (under
/// `HorizonFarTerrain`) kept every column at one block wide, as ints.
const FORMAT: i32 = 2;
const FORMAT_KEY: &str = "HorizonFarSummary";
/// The record of a chunk with no far columns (a proto chunk).
const NO_COLUMNS: &str = "HorizonFarNone";
/// The column widths a summary keeps: every saved tile stride but one, up to
/// `far_lod::SAVED_LOD_STRIDE`.
const WIDTHS: [usize; 4] = [2, 4, 8, 16];
/// Spans a column may hold in a record read back, and materials a record.
const MAX_SPANS: usize = 4096;
const MAX_MATERIALS: usize = 1 << 16;

/// A saved chunk's columns reduced to each of [`WIDTHS`] blocks wide
/// (`Column::reduced`), rows of `16 / width` from the chunk's north-west
/// corner: what a tile that coarse reads of it.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Reduced([Vec<Column>; WIDTHS.len()]);

impl Reduced {
    pub(super) fn of(columns: &[Column; 256]) -> Self {
        Self(WIDTHS.map(|width| {
            let side = 16 / width;
            (0..side * side)
                .map(|i| Column::reduced(columns, i % side * width, i / side * width, width))
                .collect()
        }))
    }
    /// The column `width` blocks wide from `x z` in the chunk, when `width`
    /// is one kept and `x z` a multiple of it.
    pub(super) fn column(&self, x: usize, z: usize, width: usize) -> Option<&Column> {
        let level = WIDTHS.iter().position(|&w| w == width)?;
        if !x.is_multiple_of(width) || !z.is_multiple_of(width) || x >= 16 || z >= 16 {
            return None;
        }
        self.0[level].get(z / width * (16 / width) + x / width)
    }
}

pub(super) enum Summary {
    /// The chunk's columns as of the timestamp asked for.
    Columns(Option<Arc<Reduced>>),
    /// None written for it, or for another save of it, or by another layout
    /// or state table.
    Missing,
}

pub(super) struct FarSummaries {
    dir: PathBuf,
    /// [`tables_hash`] of the level's block states and biomes.
    tables: i64,
    /// Open region files, `None` for one that doesn't exist.
    regions: Mutex<crate::fast_hash::FxHashMap<(i32, i32), Option<RegionFile>>>,
}

impl FarSummaries {
    pub(super) fn new(level: &Path, assets: &crate::assets::Assets, biomes: &Biomes) -> Self {
        Self {
            dir: level.join(FOLDER),
            tables: tables_hash(assets.registry, biomes),
            regions: Default::default(),
        }
    }

    /// Chunk `x z`'s summary, if one was written for its record saved at
    /// `timestamp`.
    pub(super) fn read(&self, x: i32, z: i32, timestamp: u32) -> Summary {
        let record = {
            let mut regions = self.regions.lock().unwrap_or_else(|e| e.into_inner());
            self.region(&mut regions, (x >> 5, z >> 5), false)
                .and_then(|region| region.read(ChunkPos::new(x, z)).ok().flatten())
        };
        let Some(root) = record
            .filter(|record| record.timestamp == timestamp)
            .and_then(|record| record.nbt(Limits::default()).ok())
        else {
            return Summary::Missing;
        };
        let tag = &root.tag;
        if tag.get(FORMAT_KEY) != Some(&Tag::Int(FORMAT))
            || tag.get("DataVersion") != Some(&Tag::Int(horizon_world::DATA_VERSION))
            || tag.get("States") != Some(&Tag::Long(self.tables))
        {
            return Summary::Missing;
        }
        if tag.get(NO_COLUMNS).is_some() {
            return Summary::Columns(None);
        }
        match tag.get("Columns") {
            Some(Tag::ByteArray(bytes)) => match decode_reduced(bytes) {
                Some(reduced) => Summary::Columns(Some(Arc::new(reduced))),
                None => Summary::Missing,
            },
            _ => Summary::Missing,
        }
    }

    /// Write chunk `x z`'s summary, made from its record saved at `timestamp`.
    pub(super) fn write(&self, x: i32, z: i32, timestamp: u32, reduced: Option<&Reduced>) {
        let mut fields = vec![
            (FORMAT_KEY, Tag::Int(FORMAT)),
            ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
            ("States", Tag::Long(self.tables)),
        ];
        fields.push(match reduced {
            Some(reduced) => ("Columns", Tag::ByteArray(encode_reduced(reduced))),
            None => (NO_COLUMNS, Tag::Byte(1)),
        });
        let root = NamedTag {
            name: "".into(),
            tag: Tag::compound(fields),
        };
        let Ok(record) =
            ChunkRecord::from_nbt(&root, Compression::Zlib, timestamp, Limits::default())
        else {
            return;
        };
        let mut regions = self.regions.lock().unwrap_or_else(|e| e.into_inner());
        let written = self
            .region(&mut regions, (x >> 5, z >> 5), true)
            .map(|region| region.write(ChunkPos::new(x, z), &record));
        if let Some(Err(e)) = written {
            log::debug!("Horizon: far summary of chunk {x} {z} not written: {e}");
        }
    }

    fn region<'a>(
        &self,
        regions: &'a mut crate::fast_hash::FxHashMap<(i32, i32), Option<RegionFile>>,
        key: (i32, i32),
        create: bool,
    ) -> Option<&'a mut RegionFile> {
        let path = || self.dir.join(format!("r.{}.{}.mca", key.0, key.1));
        if create && matches!(regions.get(&key), Some(None)) {
            regions.remove(&key);
        }
        if !regions.contains_key(&key) {
            if regions.len() >= OPEN_REGIONS {
                regions.clear();
            }
            if create {
                let _ = fs::create_dir_all(&self.dir);
            }
            let file = RegionFile::open_writable(path(), create).ok();
            regions.insert(key, file);
        }
        regions.get_mut(&key)?.as_mut()
    }
}

/// A hash of the block states (names and properties, by id) and the biomes
/// (names, by index) a record's numbers index, the same in every process
/// (FNV-1a).
fn tables_hash(registry: &horizon_world::registry::BlockRegistry, biomes: &Biomes) -> i64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut eat = |bytes: &[u8]| {
        // Each field ends with a byte no name holds.
        for &byte in bytes.iter().chain([&0xff]) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    for id in 0..registry.len() {
        let Some(state) = registry.state(BlockStateId(id as u32)) else {
            continue;
        };
        eat(state.name.as_bytes());
        for (key, value) in &state.properties {
            eat(key.as_bytes());
            eat(value.as_bytes());
        }
        eat(b";");
    }
    eat(b"biomes");
    for biome in &biomes.list {
        eat(biome.name.as_bytes());
    }
    hash as i64
}

fn varint(out: &mut Vec<u8>, mut value: u32) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

/// `reduced` as bytes: the materials (state, biome, light) its spans use,
/// then for each width each column's span count and spans, each as its gap
/// from the one below (zigzag; the first from 0), its height and its
/// material's index.
fn encode_reduced(reduced: &Reduced) -> Vec<u8> {
    let mut materials: crate::fast_hash::FxHashMap<(u32, u16, u16), u32> = Default::default();
    let mut order = Vec::new();
    let mut body = Vec::new();
    for column in reduced.0.iter().flatten() {
        varint(&mut body, column.spans.len() as u32);
        let mut below = 0i32;
        for span in column.spans.iter() {
            let gap = i32::from(span.min) - below;
            varint(&mut body, ((gap << 1) ^ (gap >> 31)) as u32);
            varint(
                &mut body,
                (i32::from(span.max) - i32::from(span.min)) as u32,
            );
            let material = (span.state.0, span.biome, span.light);
            let index = *materials.entry(material).or_insert_with(|| {
                order.push(material);
                order.len() as u32 - 1
            });
            varint(&mut body, index);
            below = i32::from(span.max);
        }
    }
    let mut out = Vec::with_capacity(body.len() + order.len() * 4 + 4);
    varint(&mut out, order.len() as u32);
    for (state, biome, light) in order {
        varint(&mut out, state);
        varint(&mut out, u32::from(biome));
        varint(&mut out, u32::from(light));
    }
    out.extend_from_slice(&body);
    out
}

/// `encode_reduced`'s columns, or `None` for bytes it doesn't write.
fn decode_reduced(bytes: &[u8]) -> Option<Reduced> {
    let mut bytes = bytes.iter().copied();
    let mut next = || -> Option<u32> {
        let mut value = 0u32;
        for shift in (0..35).step_by(7) {
            let byte = bytes.next()?;
            value |= u32::from(byte & 0x7f).checked_shl(shift)?;
            if byte & 0x80 == 0 {
                return Some(value);
            }
        }
        None
    };
    let count = usize::try_from(next()?)
        .ok()
        .filter(|&n| n <= MAX_MATERIALS)?;
    let mut materials = Vec::with_capacity(count);
    for _ in 0..count {
        let state = BlockStateId(next()?);
        let biome = u16::try_from(next()?).ok()?;
        let light = u16::try_from(next()?).ok()?;
        materials.push((state, biome, light));
    }
    let mut level = |width: usize| -> Option<Vec<Column>> {
        let side = 16 / width;
        let mut columns = Vec::with_capacity(side * side);
        for _ in 0..side * side {
            let count = usize::try_from(next()?).ok().filter(|&n| n <= MAX_SPANS)?;
            let mut column = Column::default();
            column.spans.reserve(count);
            let mut below = 0i32;
            for _ in 0..count {
                let gap = next()?;
                let min = below.checked_add((gap >> 1) as i32 ^ -((gap & 1) as i32))?;
                let max = min.checked_add(i32::try_from(next()?).ok()?)?;
                let (state, biome, light) = *materials.get(usize::try_from(next()?).ok()?)?;
                column.spans.push(Span {
                    min: i16::try_from(min).ok()?,
                    max: i16::try_from(max).ok()?,
                    state,
                    biome,
                    light,
                });
                below = max;
            }
            columns.push(column);
        }
        Some(columns)
    };
    let mut levels = Vec::with_capacity(WIDTHS.len());
    for width in WIDTHS {
        levels.push(level(width)?);
    }
    drop(level);
    if next().is_some() {
        return None;
    }
    Some(Reduced(levels.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_chunk_reads_back_its_summary_until_saved_again() {
        let Some((assets, biomes)) = resources() else {
            return;
        };
        let stone = assets.registry.default_state("minecraft:stone").unwrap();
        let ore = assets.registry.default_state("minecraft:iron_ore").unwrap();
        let chunk = |top: i32| {
            let mut chunk = Chunk::empty(0, 0);
            for i in 0..256i32 {
                for y in -64..=60 {
                    chunk.set(i & 15, y, i >> 4, stone.0);
                }
                // Ores and caves: spans every column but its neighbours.
                chunk.set(i & 15, (i * 7) % 100 - 60, i >> 4, ore.0);
                chunk.set(i & 15, (i * 13) % 90 - 50, i >> 4, 0);
            }
            for y in 61..top {
                chunk.set(3, y, 5, stone.0);
            }
            chunk.lit = true;
            encode(&chunk, assets.registry, &biomes)
        };
        let dir =
            std::env::temp_dir().join(format!("horizon-far-summaries-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let save = SaveDir::new(dir.clone(), LevelShape::OVERWORLD);
        save.keep_far_summaries(&assets, &biomes);
        let summaries = save.outstanding.summaries.get().unwrap().clone();
        let coarse = |save: &SaveDir| save.lod_saved(0, 0, false, &assets, &biomes);
        save.write_records("region", vec![((0, 0), Some(chunk(100)))], true)
            .unwrap();
        let timestamp = save.far_timestamp(0, 0).unwrap();
        assert!(matches!(summaries.read(0, 0, timestamp), Summary::Missing));
        // The first coarse read decodes the chunk and writes its summary: the
        // chunk's columns reduced to every width but one, exactly.
        let Some(far_lod::SavedFar::Columns(decoded)) = coarse(&save) else {
            panic!("a chunk with no summary is decoded");
        };
        assert_eq!(decoded[5 * 16 + 3].surface.unwrap().height, 100);
        let Summary::Columns(Some(written)) = summaries.read(0, 0, timestamp) else {
            panic!("the read wrote the summary it was missing");
        };
        for width in [2, 4, 8, 16] {
            for (x, z) in [(0usize, 0usize), (4, 8), (8, 0)] {
                if x.is_multiple_of(width) && z.is_multiple_of(width) {
                    let column = written.column(x, z, width).unwrap();
                    assert_eq!(*column, Column::reduced(&decoded, x, z, width));
                }
            }
        }
        assert!(written.column(0, 0, 1).is_none() && written.column(2, 0, 4).is_none());
        assert!(matches!(coarse(&save), Some(far_lod::SavedFar::Reduced(_))));
        // One block a column: the chunk itself, never its summary.
        assert!(matches!(
            save.lod_saved(0, 0, true, &assets, &biomes),
            Some(far_lod::SavedFar::Columns(_))
        ));

        // Saved again a second later (by any game): the old summary is stale,
        // and the next coarse read writes the new one.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        save.write_records("region", vec![((0, 0), Some(chunk(200)))], true)
            .unwrap();
        let timestamp = save.far_timestamp(0, 0).unwrap();
        assert!(matches!(summaries.read(0, 0, timestamp), Summary::Missing));
        let Some(far_lod::SavedFar::Columns(read)) = coarse(&save) else {
            panic!("a chunk saved again is decoded again");
        };
        assert_eq!(read[5 * 16 + 3].surface.unwrap().height, 200);
        let Summary::Columns(Some(lazy)) = summaries.read(0, 0, timestamp) else {
            panic!("a read writes the summary it was missing");
        };
        assert_eq!(*lazy, Reduced::of(&read));
        drop(save);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_summary_of_another_state_table_or_layout_reads_as_missing() {
        let Some((assets, biomes)) = resources() else {
            return;
        };
        let dir = std::env::temp_dir().join(format!(
            "horizon-far-summaries-tables-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let summaries = || FarSummaries::new(&dir, &assets, &biomes);
        let mut column = Column::default();
        column.spans.push(Span {
            min: -64,
            max: 70,
            state: BlockStateId(1),
            biome: 3,
            light: 240,
        });
        let reduced = Reduced::of(&std::array::from_fn(|_| column.clone()));
        let ours = summaries();
        ours.write(1, 2, 100, Some(&reduced));
        let Summary::Columns(Some(read)) = ours.read(1, 2, 100) else {
            panic!("the summary reads back");
        };
        assert_eq!(*read, reduced);
        // Another mod set numbers the states otherwise.
        let mut modded = summaries();
        modded.tables ^= 1;
        assert!(matches!(modded.read(1, 2, 100), Summary::Missing));
        // A record of the first layout (every column, as ints).
        let old = NamedTag {
            name: "".into(),
            tag: Tag::compound([
                ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
                ("HorizonFarTerrain", Tag::Int(1)),
                ("columns", Tag::IntArray(vec![0; 512])),
            ]),
        };
        let record = ChunkRecord::from_nbt(&old, Compression::Zlib, 100, Limits::default());
        let mut regions = ours.regions.lock().unwrap();
        let region = ours.region(&mut regions, (0, 0), true).unwrap();
        region.write(ChunkPos::new(1, 2), &record.unwrap()).unwrap();
        drop(regions);
        assert!(matches!(ours.read(1, 2, 100), Summary::Missing));
        let _ = fs::remove_dir_all(&dir);
    }

    fn resources() -> Option<(Arc<crate::assets::Assets>, Arc<Biomes>)> {
        let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES")?);
        let assets = Arc::new(crate::assets::Assets::load(&root).expect("assets"));
        let biomes = Arc::new(
            Biomes::load(&horizon_assets::resource_source::ResourceRoot::directory(
                root,
            ))
            .expect("biomes"),
        );
        Some((assets, biomes))
    }
}
