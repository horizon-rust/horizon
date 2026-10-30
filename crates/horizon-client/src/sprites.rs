//! Shared nearest-sampled sprite array for block and item textures.
use crate::Result;
use crate::labpbr::{Map, MaterialPixels};
use horizon_assets::animation::{AnimationClip, AnimationMetadata};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::BufReader,
    path::Path,
    sync::Arc,
};

#[derive(Clone)]
pub struct SpriteAnimation {
    pub layer: u32,
    frames: Arc<Vec<Vec<u8>>>,
    clip: Arc<AnimationClip>,
    /// The texture's LabPBR maps' frames (`labpbr.rs`), when they have several.
    maps: [Option<Arc<Vec<Vec<u8>>>>; Map::COUNT],
}
/// What an animated sprite shows: `frame`, mixed `millis` thousandths of the
/// way towards `next` (0 for a single frame).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameBlend {
    pub frame: usize,
    pub next: usize,
    pub millis: u32,
}
impl SpriteAnimation {
    pub fn frame(&self, tick: u64) -> usize {
        self.clip.sample(tick).frame as usize
    }
    /// `SpriteContents.AnimationState.drawToAtlas`: an animation with
    /// `interpolate` mixes the frame with the next one by
    /// `(int)(subFrame / time * 1000)` thousandths; others show the frame.
    pub fn blend(&self, tick: u64) -> FrameBlend {
        let sample = self.clip.sample(tick);
        let frame = sample.frame as usize;
        let next = sample.next_frame as usize;
        if self.clip.interpolates() && sample.interpolation_millis > 0 && next != frame {
            FrameBlend {
                frame,
                next,
                millis: sample.interpolation_millis,
            }
        } else {
            FrameBlend {
                frame,
                next: frame,
                millis: 0,
            }
        }
    }
    pub fn resized_pixels(&self, frame: usize, width: u32, height: u32) -> Vec<u8> {
        let source = self.pixels(frame);
        let side = (source.len() / 4).isqrt() as u32;
        let mut out = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                let offset = ((y * side / height * side + x * side / width) * 4) as usize;
                out.extend_from_slice(&source[offset..offset + 4]);
            }
        }
        out
    }
    pub fn pixels(&self, frame: usize) -> &[u8] {
        &self.frames[frame]
    }
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
    /// The same animation drawing `map`'s frames, frame for frame (a map with fewer frames
    /// repeats them), when the map has more than one; a still map stays in its layer.
    pub fn map(&self, map: Map) -> Option<Self> {
        let frames = self.maps[map as usize].as_ref()?;
        Some(Self {
            layer: self.layer,
            frames: Arc::new(
                (0..self.frames.len())
                    .map(|i| frames[i % frames.len()].clone())
                    .collect(),
            ),
            clip: self.clip.clone(),
            maps: Default::default(),
        })
    }
}

/// A LabPBR map of one texture: its frames' square edge and the frames.
type NativeMap = (u32, Arc<Vec<Vec<u8>>>);

#[derive(Default)]
pub struct Sprites {
    /// Edge length of every array layer and the layers' current frames, set
    /// by [`Sprites::finish`].
    pub size: u32,
    /// Smallest native layer before array-size resampling.
    pub minimum_size: u32,
    pub pixels: Vec<u8>,
    pub layers: BTreeMap<String, u32>,
    pub animations: Vec<SpriteAnimation>,
    pub opaque: Vec<bool>,
    pub translucent: Vec<bool>,
    /// Each layer's `MipmapGenerator` settings.
    pub mips: Vec<crate::mipmap::Settings>,
    /// The layers' LabPBR maps, aligned with them (`labpbr.rs`), set by [`Sprites::finish`]
    /// when a texture has one.
    pub materials: Option<MaterialPixels>,
    /// Each layer's frames at the texture's own resolution (square), and the
    /// frame shown first.
    native: Vec<(u32, Arc<Vec<Vec<u8>>>, usize)>,
    /// Each layer's normal, specular and subsurface maps, as `native`.
    native_maps: Vec<[Option<NativeMap>; Map::COUNT]>,
    /// Still textures by their pixels, maps and mip settings: mods copy vanilla's
    /// textures under their own names, and one layer serves them all (the
    /// GPU's array may hold as few as 2048).
    still: std::collections::HashMap<StillKey, u32>,
}
type StillKey = (
    Vec<u8>,
    [Option<Vec<u8>>; Map::COUNT],
    u32,
    [u32; 2],
    [bool; 4],
);
/// Edge length of vanilla's block textures: the smallest array layer.
pub const SPRITE_SIZE: u32 = 16;
/// Largest sprite array (all layers of the top mip level): resource packs
/// with bigger textures are sampled down to fit.
const MAX_ARRAY_BYTES: u64 = 256 << 20;
/// Largest LabPBR map edge: larger maps are sampled down.
const MAX_MAP_SIZE: u32 = 512;
/// Nearest-resample a square RGBA frame to another square size.
pub(crate) fn resample(frame: &[u8], from: u32, to: u32) -> Vec<u8> {
    if from == to {
        return frame.to_vec();
    }
    let mut out = Vec::with_capacity((to * to * 4) as usize);
    for row in 0..to {
        let source_row = row * from / to;
        for column in 0..to {
            let offset = ((source_row * from + column * from / to) * 4) as usize;
            out.extend_from_slice(&frame[offset..offset + 4]);
        }
    }
    out
}
impl Sprites {
    /// How many array layers the textures take.
    pub fn count(&self) -> usize {
        self.native.len()
    }
    /// Lay every layer out at one size: the largest texture's (resource packs
    /// may ship 32x, 64x... textures, like vanilla's stitched atlas), rounded
    /// up to a power of two and limited to [`MAX_ARRAY_BYTES`]. Smaller
    /// textures are scaled up by whole pixels, so UVs stay per layer.
    pub fn finish(&mut self) {
        let layers = self.native.len().max(1) as u64;
        let largest = self.native.iter().map(|n| n.0).max().unwrap_or(SPRITE_SIZE);
        self.minimum_size = self.native.iter().map(|n| n.0).min().unwrap_or(SPRITE_SIZE);
        let mut size = largest.max(SPRITE_SIZE).next_power_of_two();
        while size > SPRITE_SIZE && layers * u64::from(size * size) * 4 > MAX_ARRAY_BYTES {
            size /= 2;
        }
        let mut resized = Vec::with_capacity(self.native.len());
        self.pixels.clear();
        for (from, frames, initial) in &self.native {
            let frames: Arc<Vec<Vec<u8>>> =
                Arc::new(frames.iter().map(|f| resample(f, *from, size)).collect());
            self.pixels.extend_from_slice(&frames[*initial]);
            resized.push(frames);
        }
        // Each map at its own size, a power of two (scaled up by whole pixels, as layers are).
        let map_size = |from: u32| from.next_power_of_two().clamp(1, MAX_MAP_SIZE);
        let maps: Vec<[Option<Arc<Vec<Vec<u8>>>>; Map::COUNT]> = self
            .native_maps
            .iter()
            .map(|maps| {
                maps.each_ref().map(|map| {
                    map.as_ref().map(|(from, frames)| {
                        let to = map_size(*from);
                        Arc::new(frames.iter().map(|f| resample(f, *from, to)).collect())
                    })
                })
            })
            .collect();
        for animation in &mut self.animations {
            let layer = animation.layer as usize;
            animation.frames = resized[layer].clone();
            animation.maps = maps.get(layer).map_or_else(Default::default, |m| {
                m.clone().map(|frames| frames.filter(|f| f.len() > 1))
            });
        }
        self.materials = maps.iter().any(|m| m.iter().any(Option::is_some)).then(|| {
            let mut out = MaterialPixels::default();
            for map in Map::ALL {
                let k = map as usize;
                let set = &mut out.maps[k];
                for (layer, (_, _, initial)) in self.native.iter().enumerate() {
                    let (Some(frames), Some((from, _))) = (
                        maps.get(layer).and_then(|m| m[k].as_ref()),
                        self.native_maps.get(layer).and_then(|m| m[k].as_ref()),
                    ) else {
                        set.index.push(crate::labpbr::NO_MAP);
                        continue;
                    };
                    let edge = map_size(*from);
                    let class = match set.classes.iter().position(|c| c.size == edge) {
                        Some(class) => class,
                        None => {
                            set.classes.push(crate::labpbr::MapClass {
                                size: edge,
                                ..Default::default()
                            });
                            set.classes.len() - 1
                        }
                    };
                    let c = &mut set.classes[class];
                    set.index
                        .push(((class as u32) << 16) | c.layers.len() as u32);
                    c.layers.push(layer as u32);
                    c.pixels.extend_from_slice(&frames[initial % frames.len()]);
                    c.mips.push(crate::mipmap::Settings {
                        native_size: *from,
                        strategy: crate::mipmap::Strategy::Data,
                        ..Default::default()
                    });
                }
                // Smallest first: classes are found by size, so their order is free.
                let mut order: Vec<usize> = (0..set.classes.len()).collect();
                order.sort_by_key(|&c| set.classes[c].size);
                let mut rank = vec![0u32; order.len()];
                for (new, &old) in order.iter().enumerate() {
                    rank[old] = new as u32;
                }
                for entry in &mut set.index {
                    if *entry != crate::labpbr::NO_MAP {
                        *entry = (rank[(*entry >> 16) as usize] << 16) | (*entry & 0xffff);
                    }
                }
                let mut classes: Vec<_> = std::mem::take(&mut set.classes)
                    .into_iter()
                    .map(Some)
                    .collect();
                set.classes = order.iter().map(|&c| classes[c].take().unwrap()).collect();
            }
            out
        });
        self.size = size;
    }
    /// The layer of a texture, added on first use. A texture that is missing
    /// or does not load (a resource pack's filter or a broken file) becomes
    /// vanilla's missing texture, as in a stitched atlas.
    pub fn insert(&mut self, root: &Path, name: &str) -> Result<u32> {
        if let Some(layer) = self.layers.get(name) {
            return Ok(*layer);
        }
        match self.load(root, name) {
            Ok(layer) => Ok(layer),
            Err(e) => {
                // `MissingTextureAtlasSprite.getLocation()`: vanilla models name the
                // missing texture on purpose (block/air.json's particle), #853.
                if name != "minecraft:missingno" {
                    log::warn!("Horizon: using the missing texture for {name}: {e}");
                }
                // MissingTextureAtlasSprite.generateMissingImage: 16x16,
                // magenta and black quarters.
                let frame: Vec<u8> = (0..SPRITE_SIZE * SPRITE_SIZE)
                    .flat_map(|i| {
                        let (x, y) = (i % SPRITE_SIZE, i / SPRITE_SIZE);
                        if (y < SPRITE_SIZE / 2) ^ (x < SPRITE_SIZE / 2) {
                            [248, 0, 248, 255]
                        } else {
                            [0, 0, 0, 255]
                        }
                    })
                    .collect();
                let layer = u32::try_from(self.native.len())?;
                self.opaque.push(true);
                self.translucent.push(false);
                self.mips.push(crate::mipmap::Settings::default());
                self.layers.insert(name.to_owned(), layer);
                self.native.push((SPRITE_SIZE, Arc::new(vec![frame]), 0));
                self.native_maps.push(Default::default());
                Ok(layer)
            }
        }
    }
    fn load(&mut self, root: &Path, name: &str) -> Result<u32> {
        let (namespace, path) = name.split_once(':').ok_or("invalid sprite identifier")?;
        self.load_file(
            name,
            &root.join(format!("assets/{namespace}/textures/{path}.png")),
        )
    }
    /// The layer of the texture at `path`, known as `name` (connected textures' tiles live
    /// outside `textures/`), added on first use.
    pub fn insert_file(&mut self, name: &str, path: &Path) -> Result<u32> {
        if let Some(layer) = self.layers.get(name) {
            return Ok(*layer);
        }
        self.load_file(name, path)
    }
    fn load_file(&mut self, name: &str, path: &Path) -> Result<u32> {
        let metadata = match fs::read_to_string(path.with_extension("png.mcmeta")) {
            Ok(text) => Some(serde_json::from_str::<
                BTreeMap<String, Box<serde_json::value::RawValue>>,
            >(&text)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let animation = metadata
            .as_ref()
            .and_then(|v| v.get("animation"))
            .map(|v| AnimationMetadata::from_json(v.get()))
            .transpose()?;
        let (image_width, image_height, pixels) = read_rgba(path)?;
        let (width, height) = if let Some(animation) = &animation {
            let [width, height] =
                animation.frame_size(i32::try_from(image_width)?, i32::try_from(image_height)?);
            (u32::try_from(width)?, u32::try_from(height)?)
        } else {
            (image_width, image_height)
        };
        if width == 0 || height == 0 || image_width % width != 0 || image_height % height != 0 {
            return Err(format!("sprite {name} has invalid frame dimensions").into());
        }
        let layer = u32::try_from(self.native.len())?;
        let columns = image_width / width;
        let rows = image_height / height;
        let count = columns * rows;
        // Layers are square; a frame of another shape is nearest-resampled to
        // its longer side so UVs (normalized per layer) are unchanged.
        let side = width.max(height);
        let frames = split_frames(&pixels, image_width, [width, height], count);
        let maps = Map::ALL.map(|map| load_map(path, map, [columns, rows]));
        let settings = crate::mipmap::Settings {
            native_size: side,
            strategy: texture_section(metadata.as_ref())
                .as_ref()
                .and_then(|t| t["mipmap_strategy"].as_str())
                .and_then(crate::mipmap::Strategy::parse)
                .unwrap_or_default(),
            alpha_cutoff_bias: texture_section(metadata.as_ref())
                .as_ref()
                .and_then(|t| t["alpha_cutoff_bias"].as_f64())
                .unwrap_or(0.0) as f32,
            has_transparent: pixels.as_chunks::<4>().0.iter().any(|p| p[3] == 0),
            item: name
                .split_once(':')
                .is_some_and(|(_, p)| p.starts_with("item/")),
        };
        let opaque = pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255);
        let translucent = pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[3] != 0 && p[3] != 255);
        let still = (animation.is_none() && frames.len() == 1).then(|| {
            (
                frames[0].clone(),
                maps.each_ref()
                    .map(|m| m.as_ref().map(|(_, frames)| frames[0].clone())),
                side,
                [
                    settings.strategy as u32,
                    settings.alpha_cutoff_bias.to_bits(),
                ],
                [settings.has_transparent, settings.item, opaque, translucent],
            )
        });
        if let Some(&same) = still.as_ref().and_then(|k| self.still.get(k)) {
            self.layers.insert(name.to_owned(), same);
            return Ok(same);
        }
        if let Some(key) = still {
            self.still.insert(key, layer);
        }
        let frames = Arc::new(frames);
        let mut initial = 0;
        if let Some(clip) = animation
            .as_ref()
            .map(|m| m.clip(count))
            .transpose()?
            .flatten()
        {
            initial = clip.sample(0).frame as usize;
            self.animations.push(SpriteAnimation {
                layer,
                frames: frames.clone(),
                clip: Arc::new(clip),
                maps: Default::default(),
            });
        }
        self.mips.push(settings);
        self.opaque.push(opaque);
        self.translucent.push(translucent);
        self.layers.insert(name.to_owned(), layer);
        self.native.push((side, frames, initial));
        self.native_maps.push(maps);
        Ok(layer)
    }
}

/// A PNG as RGBA rows: its width, height and pixels.
fn read_rgba(path: &Path) -> Result<(u32, u32, Vec<u8>)> {
    let mut decoder = png::Decoder::new(BufReader::new(File::open(path)?));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info()?;
    let mut bytes = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or("sprite exceeds decoder limits")?
    ];
    let info = reader.next_frame(&mut bytes)?;
    let channels = info.color_type.samples();
    let mut pixels = Vec::with_capacity(info.width as usize * info.height as usize * 4);
    for pixel in bytes[..info.buffer_size()].chunks_exact(channels) {
        let rgba = match info.color_type {
            png::ColorType::Rgba => [pixel[0], pixel[1], pixel[2], pixel[3]],
            png::ColorType::Rgb => [pixel[0], pixel[1], pixel[2], 255],
            png::ColorType::Grayscale => [pixel[0], pixel[0], pixel[0], 255],
            png::ColorType::GrayscaleAlpha => [pixel[0], pixel[0], pixel[0], pixel[1]],
            _ => return Err("unsupported expanded sprite format".into()),
        };
        pixels.extend(rgba);
    }
    Ok((info.width, info.height, pixels))
}

/// The first `count` frames of `[width, height]` of an image `image_width` wide, row by row,
/// each nearest-resampled to a square of its longer side.
fn split_frames(
    pixels: &[u8],
    image_width: u32,
    [width, height]: [u32; 2],
    count: u32,
) -> Vec<Vec<u8>> {
    let columns = image_width / width;
    let side = width.max(height);
    (0..count)
        .map(|index| {
            let x = index % columns * width;
            let y = index / columns * height;
            let mut frame = Vec::with_capacity((side * side * 4) as usize);
            for row in 0..side {
                let source_row = y + row * height / side;
                for column in 0..side {
                    let offset =
                        ((source_row * image_width + x + column * width / side) * 4) as usize;
                    frame.extend_from_slice(&pixels[offset..offset + 4]);
                }
            }
            frame
        })
        .collect()
}

/// The `map` beside the texture at `path` (`stone.png`: `stone_n.png`), cut into frames like
/// the texture's `[columns, rows]` frames when its size divides into them (else by its own
/// `.mcmeta`, else whole), as LabPBR packs draw animated maps. `None` without a map; a map
/// that does not load is skipped with a warning.
fn load_map(path: &Path, map: Map, [columns, rows]: [u32; 2]) -> Option<NativeMap> {
    let stem = path.file_stem()?.to_str()?;
    let map_path = path.with_file_name(format!("{stem}{}.png", map.suffix()));
    if !map_path.is_file() {
        return None;
    }
    let (width, height, pixels) = match read_rgba(&map_path) {
        Ok(image) => image,
        Err(e) => {
            log::warn!(
                "Horizon: skipping the material map {}: {e}",
                map_path.display()
            );
            return None;
        }
    };
    let own = || {
        let text = fs::read_to_string(map_path.with_extension("png.mcmeta")).ok()?;
        let v: BTreeMap<String, Box<serde_json::value::RawValue>> =
            serde_json::from_str(&text).ok()?;
        let animation = AnimationMetadata::from_json(v.get("animation")?.get()).ok()?;
        let [w, h] = animation.frame_size(i32::try_from(width).ok()?, i32::try_from(height).ok()?);
        Some([u32::try_from(w).ok()?, u32::try_from(h).ok()?])
    };
    let frame = if width % columns == 0 && height % rows == 0 {
        [width / columns, height / rows]
    } else {
        own().unwrap_or([width, height])
    };
    if frame[0] == 0 || frame[1] == 0 || width % frame[0] != 0 || height % frame[1] != 0 {
        log::warn!(
            "Horizon: skipping the material map {}: invalid frame size",
            map_path.display()
        );
        return None;
    }
    let count = (width / frame[0]) * (height / frame[1]);
    Some((
        frame[0].max(frame[1]),
        Arc::new(split_frames(&pixels, width, frame, count)),
    ))
}

/// SpriteContents: the `.mcmeta` `texture` section (TextureMetadataSection's
/// mipmap strategy and alpha cutoff bias).
fn texture_section(
    metadata: Option<&BTreeMap<String, Box<serde_json::value::RawValue>>>,
) -> Option<serde_json::Value> {
    metadata
        .and_then(|v| v.get("texture"))
        .and_then(|v| serde_json::from_str(v.get()).ok())
}

#[cfg(test)]
pub(crate) fn test_animation(size: u32, layer: u32) -> SpriteAnimation {
    test_clip(size, layer, r#"{"frametime":1,"frames":[0,1]}"#)
}
#[cfg(test)]
pub(crate) fn test_clip(size: u32, layer: u32, metadata: &str) -> SpriteAnimation {
    let frame = |blue: u8| {
        (0..size * size)
            .flat_map(|i| [(i % size) as u8, (i / size) as u8, blue, 255])
            .collect()
    };
    test_frames(layer, vec![frame(0), frame(99)], metadata)
}
#[cfg(test)]
pub(crate) fn test_frames(layer: u32, frames: Vec<Vec<u8>>, metadata: &str) -> SpriteAnimation {
    let metadata = AnimationMetadata::from_json(metadata).unwrap();
    SpriteAnimation {
        layer,
        clip: Arc::new(metadata.clip(frames.len() as u32).unwrap().unwrap()),
        frames: Arc::new(frames),
        maps: Default::default(),
    }
}

/// How many reduced copies a sprite atlas keeps: the Mipmap Levels option plus the
/// resampling-only levels an upscaled pack needs, within what the atlas holds.
pub(crate) fn sprite_mipmap_limit(requested: u32, size: u32, minimum: u32, available: u32) -> u32 {
    // Only the atlas native minimum determines how many resampling-only mips to skip.
    let upscale = (size / minimum.max(1)).max(1).ilog2();
    requested.saturating_add(upscale).min(available - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mipmap_option_keeps_real_16px_reductions() {
        for levels in 0..=4 {
            assert_eq!(sprite_mipmap_limit(levels, 32, 16, 6), levels + 1);
            assert_eq!(sprite_mipmap_limit(levels, 16, 16, 5), levels);
        }
    }
    #[test]
    fn identical_still_textures_share_a_layer() {
        let root =
            std::env::temp_dir().join(format!("horizon-sprite-share-{}", std::process::id()));
        let directory = root.join("assets/a/textures/block");
        fs::create_dir_all(&directory).unwrap();
        let write = |name: &str, red: u8| {
            let mut encoder =
                png::Encoder::new(File::create(directory.join(name)).unwrap(), 16, 16);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let pixels: Vec<u8> = (0..256).flat_map(|i| [red, i as u8, 0, 255]).collect();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        };
        write("one.png", 10);
        write("copy.png", 10);
        write("other.png", 20);
        let mut sprites = Sprites::default();
        let one = sprites.insert(&root, "a:block/one").unwrap();
        let copy = sprites.insert(&root, "a:block/copy").unwrap();
        let other = sprites.insert(&root, "a:block/other").unwrap();
        // The same pixels as an item sprite mip differently: a layer of its own.
        fs::create_dir_all(root.join("assets/a/textures/item")).unwrap();
        fs::copy(
            directory.join("one.png"),
            root.join("assets/a/textures/item/one.png"),
        )
        .unwrap();
        let item = sprites.insert(&root, "a:item/one").unwrap();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!((one, copy, other, item), (0, 0, 1, 2));
        assert_eq!((sprites.count(), sprites.layers.len()), (3, 4));
        assert_eq!((sprites.opaque.len(), sprites.mips.len()), (3, 3));
        sprites.finish();
        assert_eq!(sprites.pixels.len(), 3 * 16 * 16 * 4);
    }
    #[test]
    fn a_32px_only_atlas_does_not_add_an_extra_mipmap() {
        let mut sprites = Sprites {
            native: vec![
                (32, Arc::new(vec![vec![255; 32 * 32 * 4]]), 0),
                (32, Arc::new(vec![vec![128; 32 * 32 * 4]]), 0),
            ],
            ..Default::default()
        };
        sprites.finish();
        assert_eq!((sprites.size, sprites.minimum_size), (32, 32));
        for level in 0..=4 {
            assert_eq!(
                sprite_mipmap_limit(level, sprites.size, sprites.minimum_size, 6),
                level
            );
        }
        sprites
            .native
            .push((16, Arc::new(vec![vec![255; 16 * 16 * 4]]), 0));
        sprites.finish();
        assert_eq!((sprites.size, sprites.minimum_size), (32, 16));
        assert_eq!(
            sprite_mipmap_limit(0, sprites.size, sprites.minimum_size, 6),
            1
        );
    }

    #[test]
    fn hud_animation_resampling_uses_the_actual_frame_size() {
        for size in [16, 32, 64] {
            let animation = test_animation(size, 0);
            let resized = animation.resized_pixels(1, 16, 16);
            let last = (15 * size / 16) as u8;
            assert_eq!(&resized[15 * 4..16 * 4], &[last, 0, 99, 255]);
            assert_eq!(&resized[255 * 4..256 * 4], &[last, last, 99, 255]);
        }
    }

    #[test]
    fn native_fire_sprites_register_full_animation_sequences() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root
            .join("assets/minecraft/textures/block/fire_0.png")
            .exists()
        {
            return;
        }
        let mut sprites = Sprites::default();
        for name in ["fire_0", "fire_1", "soul_fire_0", "soul_fire_1"] {
            let layer = sprites
                .insert(&root, &format!("minecraft:block/{name}"))
                .unwrap();
            let animation = sprites
                .animations
                .iter()
                .find(|a| a.layer == layer)
                .expect("fire must be animated");
            assert_ne!(
                animation.pixels(animation.frame(0)),
                animation.pixels(animation.frame(1))
            );
            assert_ne!(
                animation.resized_pixels(animation.frame(0), 16, 16),
                animation.resized_pixels(animation.frame(1), 16, 16)
            );
            assert_eq!(
                animation.pixels(animation.frame(0)).len(),
                (SPRITE_SIZE * SPRITE_SIZE * 4) as usize
            );
        }
    }

    #[test]
    fn native_32_pixel_animation_frames_keep_every_texel() {
        let root = std::env::temp_dir().join(format!(
            "horizon-sprite-detail-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let directory = root.join("assets/test/textures");
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("flow.png");
        let mut pixels = Vec::new();
        for y in 0..64u8 {
            for x in 0..32u8 {
                pixels.extend([x, y, 0, 255]);
            }
        }
        let mut encoder = png::Encoder::new(File::create(&path).unwrap(), 32, 64);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        fs::write(path.with_extension("png.mcmeta"), r#"{"animation":{}}"#).unwrap();
        let mut sprites = Sprites::default();
        sprites.insert(&root, "test:flow").unwrap();
        sprites.finish();
        let size = sprites.size;
        let initial = sprites.pixels.clone();
        let animation = sprites.animations[0].clone();
        fs::remove_dir_all(root).unwrap();
        assert_eq!(
            size, 32,
            "half a flowing-water frame must retain 16 texels, not 8"
        );
        assert_eq!(initial, pixels[..32 * 32 * 4]);
        assert_eq!(animation.pixels(animation.frame(1)), &pixels[32 * 32 * 4..]);
    }

    #[test]
    fn animation_boundaries_and_repeated_frames_do_not_depend_on_render_rate() {
        let animation = SpriteAnimation {
            layer: 0,
            frames: Arc::new(vec![vec![], vec![], vec![]]),
            clip: Arc::new(AnimationMetadata::from_json(r#"{"frames":[{"index":2,"time":8},{"index":1,"time":3},{"index":1,"time":2},0]}"#).unwrap().clip(3).unwrap().unwrap()),
            maps: Default::default(),
        };
        for (tick, frame) in [
            (0, 2),
            (7, 2),
            (8, 1),
            (10, 1),
            (11, 1),
            (12, 1),
            (13, 0),
            (14, 2),
            (29, 2),
            (u64::MAX, 2),
        ] {
            assert_eq!(animation.frame(tick), frame);
        }
    }

    #[test]
    fn interpolated_animations_blend_towards_the_next_frame() {
        let animation = test_clip(16, 0, r#"{"frametime":4,"interpolate":true}"#);
        let blend = |frame, next, millis| FrameBlend {
            frame,
            next,
            millis,
        };
        for (tick, expected) in [
            (0, blend(0, 0, 0)),
            (1, blend(0, 1, 250)),
            (3, blend(0, 1, 750)),
            (4, blend(1, 1, 0)),
            (5, blend(1, 0, 250)),
            (8, blend(0, 0, 0)),
        ] {
            assert_eq!(animation.blend(tick), expected, "tick {tick}");
        }
        let stepped = test_clip(16, 0, r#"{"frametime":4}"#);
        assert_eq!(stepped.blend(1), blend(0, 0, 0));
        assert_eq!(stepped.blend(5), blend(1, 1, 0));
    }

    fn write_png(path: &Path, width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 4]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut encoder = png::Encoder::new(File::create(path).unwrap(), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let pixels: Vec<u8> = (0..width * height)
            .flat_map(|i| pixel(i % width, i / width))
            .collect();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
    }

    /// A still texture with a 32x normal map, one with only an 8x subsurface tint, one without
    /// maps and an animated one whose specular map has its frames: the maps sit at their textures' layers, at the array's
    /// size, neutral where a texture has none, and the animation carries its map's frames.
    #[test]
    fn labpbr_maps_line_up_with_their_layers() {
        let root = std::env::temp_dir().join(format!("horizon-sprite-maps-{}", std::process::id()));
        let dir = root.join("assets/a/textures/block");
        write_png(&dir.join("lit.png"), 16, 16, |_, _| [10, 10, 10, 255]);
        write_png(&dir.join("lit_n.png"), 32, 32, |x, _| {
            [x as u8 * 8, 0, 255, 200]
        });
        write_png(&dir.join("plain.png"), 16, 16, |_, _| [20, 20, 20, 255]);
        write_png(&dir.join("leaf.png"), 16, 16, |_, _| [30, 30, 30, 255]);
        write_png(&dir.join("leaf_sss.png"), 8, 8, |_, _| [200, 230, 10, 255]);
        write_png(&dir.join("flow.png"), 16, 32, |_, y| {
            [30, (y / 16) as u8, 0, 255]
        });
        fs::write(dir.join("flow.png.mcmeta"), r#"{"animation":{}}"#).unwrap();
        write_png(&dir.join("flow_s.png"), 16, 32, |_, y| {
            [0, 0, 0, (y / 16) as u8 * 100]
        });
        // The same pixels as `plain` but with a map: a layer of its own.
        write_png(&dir.join("bumpy.png"), 16, 16, |_, _| [20, 20, 20, 255]);
        write_png(&dir.join("bumpy_n.png"), 16, 16, |_, _| [1, 2, 3, 4]);
        let mut sprites = Sprites::default();
        let layers = ["lit", "plain", "flow", "bumpy", "leaf"]
            .map(|n| sprites.insert(&root, &format!("a:block/{n}")).unwrap());
        sprites.finish();
        fs::remove_dir_all(&root).unwrap();
        assert_eq!(layers, [0, 1, 2, 3, 4]);
        assert_eq!(sprites.size, 16);
        let m = sprites.materials.as_ref().expect("maps");
        let has = |map| layers.map(|l| m.has(l, map));
        assert_eq!(has(Map::Normal), [true, false, false, true, false]);
        assert_eq!(has(Map::Specular), [false, false, true, false, false]);
        assert_eq!(has(Map::Subsurface), [false, false, false, false, true]);
        let tints = &m.maps[Map::Subsurface as usize];
        assert_eq!(tints.slot(4), Some((0, 0)));
        assert_eq!(tints.classes[0].size, 8);
        assert_eq!(&tints.classes[0].pixels[..4], &[200, 230, 10, 255]);
        // Each map at its own size, smallest class first: bumpy's 16x, then lit's 32x.
        let normals = &m.maps[Map::Normal as usize];
        assert_eq!(
            normals.classes.iter().map(|c| c.size).collect::<Vec<_>>(),
            [16, 32]
        );
        assert_eq!(
            (normals.slot(0), normals.slot(3)),
            (Some((1, 0)), Some((0, 0)))
        );
        assert_eq!(&normals.classes[1].pixels[4..8], &[8, 0, 255, 200]);
        assert_eq!(&normals.classes[0].pixels[..4], &[1, 2, 3, 4]);
        assert_eq!(normals.classes[1].mips[0].native_size, 32);
        assert_eq!(
            normals.classes[1].mips[0].strategy,
            crate::mipmap::Strategy::Data
        );
        assert_eq!(normals.bytes(), (16 * 16 + 32 * 32) * 4);
        let specular = &m.maps[Map::Specular as usize];
        assert_eq!(specular.slot(2), Some((0, 0)));
        assert_eq!(&specular.classes[0].pixels[..4], &[0, 0, 0, 0]);
        let animation = &sprites.animations[0];
        assert!(animation.map(Map::Normal).is_none());
        let specular = animation.map(Map::Specular).expect("animated specular map");
        assert_eq!(specular.frame_count(), 2);
        assert_eq!(specular.pixels(1)[3], 100);
        assert_eq!(specular.frame(1), animation.frame(1));
    }

    /// SPBR, when it is unpacked in `.local/spbr/x`: its maps load beside vanilla's textures.
    #[test]
    fn spbr_maps_load_when_present() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/spbr/x");
        if !root
            .join("assets/minecraft/textures/block/stone_n.png")
            .exists()
        {
            return;
        }
        let mut sprites = Sprites::default();
        for name in [
            "stone",
            "oak_leaves",
            "glowstone",
            "iron_block",
            "nether_portal",
        ] {
            sprites
                .insert(&root, &format!("minecraft:block/{name}"))
                .unwrap();
        }
        sprites.finish();
        let m = sprites.materials.as_ref().expect("SPBR has maps");
        for map in [Map::Normal, Map::Specular] {
            assert!((0..5).all(|l| m.has(l, map)));
        }
        // LabPBR has no backlight tint.
        assert!((0..5).all(|l| !m.has(l, Map::Subsurface)));
        let texels = |map: Map, layer: u32| -> Vec<[u8; 4]> {
            let set = &m.maps[map as usize];
            let (class, local) = set.slot(layer).unwrap();
            let c = &set.classes[class];
            let bytes = (c.size * c.size * 4) as usize;
            c.pixels[local as usize * bytes..(local as usize + 1) * bytes]
                .as_chunks::<4>()
                .0
                .to_vec()
        };
        // Glowstone emits, iron is a metal (SPBR: 255, the albedo as F0), stone has relief.
        assert!(texels(Map::Specular, 2).iter().any(|t| t[3] < 255));
        assert!(texels(Map::Specular, 3).iter().any(|t| t[1] >= 230));
        assert!(texels(Map::Normal, 0).iter().any(|t| t[3] < 255));
        // SPBR is 16x: its stone map stays 16x.
        let normals = &m.maps[Map::Normal as usize];
        assert_eq!(normals.classes[normals.slot(0).unwrap().0].size, 16);
        assert!(sprites.animations[0].map(Map::Normal).is_some());
    }
}
