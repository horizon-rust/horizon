//! Fonts (FontManager / FontSet): `font/default.json` and every other font a
//! text component names (`minecraft:alt`, `minecraft:uniform`, a pack's own),
//! each a list of providers tried in order: `space`, `bitmap` (sheets in the
//! shared atlas, or rasterized on demand when a pack's sheet is not in it),
//! `unihex` (GNU Unifont) and `ttf` (TrueTypeGlyphProvider, rasterized with
//! ab_glyph). Glyphs drawn at run time go into a reserved atlas region.
//! Vanilla glyph metrics (ascent, oversampling, advances, bold/shadow
//! offsets), styled drawing and StringSplitter-style line wrapping.
use crate::{
    hud::Rect,
    resource_source::ResourceSource,
    text::{Span, Style, Text, legacy_format},
    zip::Zip,
};
use ab_glyph::{Font as _, ScaleFont as _};
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::{HashMap, VecDeque},
    sync::Arc,
};

/// `FontDescription.DEFAULT`.
pub const DEFAULT: &str = "minecraft:default";

/// A glyph ready to draw: atlas UVs and its box relative to the pen
/// position (GlyphBitmap left/top/right/bottom), in GUI pixels.
#[derive(Clone, Copy, Debug, Default)]
pub struct Glyph {
    pub rect: Option<Rect>,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub advance: f32,
    /// Shadow and bold offset (1 for bitmap and TrueType glyphs, 0.5 for unihex).
    pub offset: f32,
}
/// Pixels to copy into the shared atlas before the next HUD draw.
#[derive(Clone, Debug)]
pub struct Patch {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
#[derive(Clone, Copy)]
struct HexGlyph {
    /// Rows left-aligned in 32 bits.
    rows: [u32; 16],
    left: u32,
    right: u32,
}
#[derive(Clone, Copy)]
struct HexEntry {
    codepoint: u32,
    offset: u32,
    left: u8,
    right: u8,
    bytes_per_row: u8,
}
/// Compact Unihex table: sorted codepoint index + tightly packed 1/2/3/4-byte rows (~2.6 MB
/// for full GNU Unifont instead of ~10 MB in a `HashMap<u32, HexGlyph>`).
struct HexTable {
    entries: Vec<HexEntry>,
    rows: Vec<u8>,
}
impl HexTable {
    fn get(&self, codepoint: u32) -> Option<HexGlyph> {
        let idx = self
            .entries
            .binary_search_by_key(&codepoint, |e| e.codepoint)
            .ok()?;
        let e = self.entries[idx];
        let bpr = e.bytes_per_row as usize;
        let start = e.offset as usize;
        let mut rows = [0u32; 16];
        for (r, row) in rows.iter_mut().enumerate() {
            let mut bytes = [0u8; 4];
            let src = start + r * bpr;
            bytes[..bpr].copy_from_slice(&self.rows[src..src + bpr]);
            *row = u32::from_be_bytes(bytes);
        }
        Some(HexGlyph {
            rows,
            left: u32::from(e.left),
            right: u32::from(e.right),
        })
    }
    fn advance(&self, codepoint: u32) -> Option<f32> {
        let idx = self
            .entries
            .binary_search_by_key(&codepoint, |e| e.codepoint)
            .ok()?;
        let e = self.entries[idx];
        let width = u32::from(e.right).saturating_sub(u32::from(e.left)) + 1;
        Some((width / 2 + 1) as f32)
    }
}
/// A provider's `filter` (`FontOption.Filter`): the options it needs.
#[derive(Clone, Copy, Default)]
struct Filter {
    uniform: Option<bool>,
    jp: Option<bool>,
}
impl Filter {
    fn of(provider: &serde_json::Value) -> Self {
        let f = &provider["filter"];
        Self {
            uniform: f["uniform"].as_bool(),
            jp: f["jp"].as_bool(),
        }
    }
    fn passes(self, uniform: bool, japanese: bool) -> bool {
        self.uniform.is_none_or(|v| v == uniform) && self.jp.is_none_or(|v| v == japanese)
    }
}
enum StripPixels {
    Raw(Vec<u8>),
    Lz4 { len: usize, compressed: Vec<u8> },
}
/// One cell-row strip of an off-atlas bitmap font sheet, LZ4-compressed when worthwhile so
/// custom HUD/font packs do not keep megabytes of raw uncompressed RGBA sheets in RAM.
struct SheetStrip {
    stride: u32,
    pixels: StripPixels,
}
impl SheetStrip {
    fn new(stride: u32, raw: &[u8]) -> Self {
        let pixels = if raw.len() > 512 {
            let mut compressed = lz4_flex::compress_prepend_size(raw);
            if compressed.len() < raw.len() {
                compressed.shrink_to_fit();
                StripPixels::Lz4 {
                    len: raw.len(),
                    compressed,
                }
            } else {
                StripPixels::Raw(raw.to_vec())
            }
        } else {
            StripPixels::Raw(raw.to_vec())
        };
        Self { stride, pixels }
    }
    fn pixels(&self) -> Cow<'_, [u8]> {
        match &self.pixels {
            StripPixels::Raw(raw) => Cow::Borrowed(raw),
            StripPixels::Lz4 { len, compressed } => Cow::Owned(
                lz4_flex::decompress_size_prepended(compressed)
                    .ok()
                    .filter(|p| p.len() == *len)
                    .unwrap_or_else(|| vec![0; *len]),
            ),
        }
    }
}
/// A bitmap glyph whose sheet is not in the atlas: the cell to copy from its row strip.
struct SheetCell {
    strip: Arc<SheetStrip>,
    x: u32,
    w: u32,
    h: u32,
    glyph: Glyph,
}
impl SheetCell {
    /// The cell's RGBA rows, or `None` where the cell reaches past the strip.
    fn pixels(&self) -> Option<Vec<u8>> {
        let pixels = self.strip.pixels();
        let stride = self.strip.stride as usize;
        let (x, w) = (self.x as usize, self.w as usize);
        if x + w > stride {
            return None;
        }
        let mut rgba = Vec::with_capacity(w * self.h as usize * 4);
        for y in 0..self.h as usize {
            let at = (y * stride + x) * 4;
            rgba.extend_from_slice(pixels.get(at..at + w * 4)?);
        }
        Some(rgba)
    }
}
enum BitmapGlyph {
    Atlas(Glyph),
    Sheet(SheetCell),
}
/// `TrueTypeGlyphProvider`.
struct TrueType {
    font: ab_glyph::FontVec,
    /// Sorted skipped codepoints from the provider's `skip` list (usually empty).
    skip: Vec<char>,
    /// FT_Set_Pixel_Sizes: `size * oversample` pixels to the em.
    scale: ab_glyph::PxScale,
    oversample: f32,
    /// FT_Set_Transform's delta, in oversampled pixels (y down).
    shift: (f32, f32),
}
enum UnihexState {
    Unloaded {
        source: ResourceSource,
        def: serde_json::Value,
    },
    Loaded(Arc<HexTable>),
}
enum Provider {
    Space(crate::fast_hash::FxHashMap<char, f32>),
    Bitmap(Filter, crate::fast_hash::FxHashMap<char, BitmapGlyph>),
    /// The definition while lazy, then the shared compact glyph table once read.
    Unihex(Filter, UnihexState),
    TrueType(Filter, Box<TrueType>),
}
/// Direct O(1) cache for ASCII characters (`0..128`) in a [`FontSet`]: eliminates hashing
/// on every HUD/F3/chat/hotbar glyph lookup and keeps advances packed in 512 bytes (8 cache
/// lines) for fast width measurement.
struct AsciiCache {
    glyphs: [Option<Glyph>; 128],
    /// Cached `Glyph::advance`; `f32::NAN` marks uncached slots.
    advances: [f32; 128],
}
impl AsciiCache {
    fn new() -> Self {
        Self {
            glyphs: [None; 128],
            advances: [f32::NAN; 128],
        }
    }
    fn clear(&mut self) {
        self.glyphs.fill(None);
        self.advances.fill(f32::NAN);
    }
}
/// One font (`FontSet`): its providers and the glyphs resolved so far.
struct FontSet {
    providers: Vec<Provider>,
    /// Lazily allocated O(1) ASCII cache so unused font sets pay zero heap bytes.
    ascii: Option<Box<AsciiCache>>,
    /// Non-ASCII glyphs resolved so far (Fx hasher, not SipHash).
    glyphs: crate::fast_hash::FxHashMap<char, Glyph>,
    /// `SelectedGlyphs.nonFishy` of the characters whose first glyph has a fishy advance (the
    /// others use `ascii`/`glyphs`): what obfuscated text draws.
    non_fishy: crate::fast_hash::FxHashMap<char, Glyph>,
    /// `glyphsByWidth` for obfuscated text (built on first use), by `ceil(advance)`.
    by_width: Option<crate::fast_hash::FxHashMap<i32, Vec<char>>>,
}
impl FontSet {
    #[inline]
    fn get_glyph(&self, c: char) -> Option<Glyph> {
        if (c as u32) < 128 {
            self.ascii.as_deref().and_then(|a| a.glyphs[c as usize])
        } else {
            self.glyphs.get(&c).copied()
        }
    }
    #[inline]
    fn insert_glyph(&mut self, c: char, glyph: Glyph) {
        if (c as u32) < 128 {
            let ascii = self
                .ascii
                .get_or_insert_with(|| Box::new(AsciiCache::new()));
            let idx = c as usize;
            ascii.glyphs[idx] = Some(glyph);
            ascii.advances[idx] = glyph.advance;
        } else {
            self.glyphs.insert(c, glyph);
        }
    }
    fn clear_cache(&mut self) {
        if let Some(ascii) = &mut self.ascii {
            ascii.clear();
        }
        self.glyphs.clear();
        self.non_fishy.clear();
        self.by_width = None;
    }
}
/// The atlas region glyphs are rasterized into, filled in rows.
struct Region {
    cursor: (u32, u32),
    row: u32,
    pending: Vec<Patch>,
    /// A glyph did not fit since the last `Font::begin_frame`.
    full: bool,
}
struct Cache {
    /// Every font by id, the default one first (the common lookup).
    sets: Vec<(String, FontSet)>,
    region: Region,
    /// Shared parsed Unihex tables keyed by serialized provider definition so multiple font
    /// sets referencing the same `unifont` never parse or store duplicate copies.
    unihex: HashMap<String, Arc<HexTable>>,
    /// `Font.random`: picks obfuscated glyphs.
    random: horizon_core::random::LegacyRandom,
}
pub struct Font {
    missing: Glyph,
    white: Rect,
    /// Atlas size and the reserved dynamic region (y, height).
    atlas: (u32, u32),
    region: (u32, u32),
    metrics_only: bool,
    /// Index of [`DEFAULT`] in `cache.sets`, if present.
    default_set: Option<usize>,
    cache: RefCell<Cache>,
    /// Set anew for each font and each `reset_cache` (see `epoch`).
    generation: std::cell::Cell<u64>,
    /// "Force Unicode Font": the `uniform` filter option.
    pub uniform: bool,
    /// "Japanese Glyph Variants": the `jp` filter option.
    pub japanese: bool,
}
#[inline]
fn parse_hex_digit(b: u8) -> Option<u32> {
    match b {
        b'0'..=b'9' => Some(u32::from(b - b'0')),
        b'a'..=b'f' => Some(u32::from(b - b'a' + 10)),
        b'A'..=b'F' => Some(u32::from(b - b'A' + 10)),
        _ => None,
    }
}
#[inline]
fn parse_hex_byte(hi: u8, lo: u8) -> Option<u8> {
    Some(((parse_hex_digit(hi)? << 4) | parse_hex_digit(lo)?) as u8)
}
/// Parse one `.hex` file into compact [`HexEntry`] + packed row bytes.
fn parse_hex_into(
    text: &[u8],
    entries: &mut Vec<HexEntry>,
    rows_buf: &mut Vec<u8>,
    by_cp: &mut crate::fast_hash::FxHashMap<u32, usize>,
) {
    for line in text.split(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Some(colon) = line.iter().position(|b| *b == b':') else {
            continue;
        };
        let (code, data) = (&line[..colon], &line[colon + 1..]);
        if !(4..=6).contains(&code.len()) {
            continue;
        }
        let Some(codepoint) = code
            .iter()
            .try_fold(0u32, |a, b| parse_hex_digit(*b).map(|d| (a << 4) | d))
        else {
            continue;
        };
        let bpr = match data.len() {
            32 => 1usize,
            64 => 2,
            96 => 3,
            128 => 4,
            _ => continue,
        };
        let bit_width = (bpr as u32) * 8;
        let mut row_bytes = [0u8; 64];
        let mut mask = 0u32;
        let mut ok = true;
        for r in 0..16 {
            let mut be = [0u8; 4];
            for j in 0..bpr {
                let pos = (r * bpr + j) * 2;
                match parse_hex_byte(data[pos], data[pos + 1]) {
                    Some(byte) => {
                        row_bytes[r * bpr + j] = byte;
                        be[j] = byte;
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                break;
            }
            mask |= u32::from_be_bytes(be);
        }
        if !ok {
            continue;
        }
        // LineData.calculateWidth.
        let (left, right) = if mask == 0 {
            (0u8, bit_width as u8)
        } else {
            (
                mask.leading_zeros() as u8,
                (31 - mask.trailing_zeros()) as u8,
            )
        };
        let slice = &row_bytes[..16 * bpr];
        if let Some(&idx) = by_cp.get(&codepoint) {
            let existing = &mut entries[idx];
            if existing.bytes_per_row as usize == bpr {
                let start = existing.offset as usize;
                rows_buf[start..start + slice.len()].copy_from_slice(slice);
            } else {
                existing.offset = rows_buf.len() as u32;
                existing.bytes_per_row = bpr as u8;
                rows_buf.extend_from_slice(slice);
            }
            existing.left = left;
            existing.right = right;
        } else {
            let offset = rows_buf.len() as u32;
            rows_buf.extend_from_slice(slice);
            by_cp.insert(codepoint, entries.len());
            entries.push(HexEntry {
                codepoint,
                offset,
                left,
                right,
                bytes_per_row: bpr as u8,
            });
        }
    }
}
/// UnihexProvider.readFromStream over one `.hex` file.
#[cfg(test)]
fn parse_hex(text: &[u8], out: &mut HashMap<u32, HexGlyph>) {
    let mut entries = Vec::new();
    let mut rows_buf = Vec::new();
    let mut by_cp = crate::fast_hash::FxHashMap::default();
    parse_hex_into(text, &mut entries, &mut rows_buf, &mut by_cp);
    entries.sort_unstable_by_key(|e| e.codepoint);
    let table = HexTable {
        entries,
        rows: rows_buf,
    };
    for e in &table.entries {
        if let Some(g) = table.get(e.codepoint) {
            out.insert(e.codepoint, g);
        }
    }
}
fn read_json(source: &ResourceSource, path: &str) -> Option<serde_json::Value> {
    serde_json::from_slice(&source.read(path).ok()?).ok()
}
fn resource_path(id: &str, folder: &str, ext: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("assets/{ns}/{folder}/{}{ext}", path.trim_end_matches(ext))
}
/// A bitmap provider's `height` (8 when missing or null) and `ascent`, as
/// vanilla decodes them: any JSON number taken as an integer, a fraction
/// truncated toward zero, saturated to the 32-bit range (vanilla wraps there).
/// `None` when either isn't a number or `ascent` is missing.
fn bitmap_height_ascent(p: &serde_json::Value) -> Option<(i32, i32)> {
    let int = |v: &serde_json::Value| {
        v.as_i64()
            .or_else(|| v.as_f64().map(|f| f as i64))
            .map(|v| v.clamp(i32::MIN.into(), i32::MAX.into()) as i32)
    };
    let height = if p["height"].is_null() {
        Some(8)
    } else {
        int(&p["height"])
    };
    Some((height?, int(&p["ascent"])?))
}
/// Why vanilla refuses a bitmap provider while its font file decodes
/// (`BitmapProvider.Definition`): `height` or `ascent` not a number (or no
/// `ascent`), `ascent` above `height`, or a `chars` grid that is empty or whose
/// rows differ in length, counted in code points as `String.codePoints` does
/// (a Rust `char` is one). Other providers pass.
fn bitmap_error(p: &serde_json::Value) -> Option<String> {
    if p["type"] != "bitmap" {
        return None;
    }
    let file = p["file"].as_str().unwrap_or("");
    let Some((height, ascent)) = bitmap_height_ascent(p) else {
        return Some(format!(
            "bitmap {file}: height and ascent have to be numbers"
        ));
    };
    if ascent > height {
        return Some(format!(
            "bitmap {file}: ascent {ascent} higher than height {height}"
        ));
    }
    let rows = p["chars"].as_array().map(|rows| {
        rows.iter()
            .map(|r| r.as_str().map(|s| s.chars().count()))
            .collect::<Option<Vec<_>>>()
    });
    let Some(Some(rows)) = rows else {
        return Some(format!("bitmap {file}: chars has to be a list of strings"));
    };
    match rows.first() {
        None | Some(0) => Some(format!("bitmap {file}: chars has no characters")),
        Some(&width) => rows.iter().find(|&&r| r != width).map(|r| {
            format!(
                "bitmap {file}: rows of chars have to be the same length \
                 (found: {r} code points, expected: {width})"
            )
        }),
    }
}
/// The first of a font file's `providers` that vanilla refuses, and why: it
/// then drops the whole file of that pack (`FontManager.loadResourceStack`).
pub(crate) fn font_file_error(providers: &serde_json::Value) -> Option<String> {
    providers
        .as_array()
        .into_iter()
        .flatten()
        .find_map(bitmap_error)
}
/// Providers of a font definition with references expanded, in order, with
/// their `filter` objects.
fn providers(source: &ResourceSource, id: &str, depth: u32, out: &mut Vec<serde_json::Value>) {
    if depth > 8 {
        return;
    }
    let path = resource_path(id, "font", ".json");
    let Some(def) = read_json(source, &path) else {
        return;
    };
    if let Some(error) = font_file_error(&def["providers"]) {
        log::error!("font {path} skipped: {error}");
        return;
    }
    for p in def["providers"].as_array().into_iter().flatten() {
        if p["type"] == "reference" {
            let mut nested = Vec::new();
            providers(
                source,
                p["id"].as_str().unwrap_or(""),
                depth + 1,
                &mut nested,
            );
            for mut n in nested {
                // A reference's filter applies to everything it includes.
                if let Some(f) = p["filter"].as_object() {
                    let merged = n["filter"]
                        .as_object()
                        .cloned()
                        .unwrap_or_default()
                        .into_iter()
                        .chain(f.clone())
                        .collect();
                    n["filter"] = serde_json::Value::Object(merged);
                }
                out.push(n);
            }
        } else {
            out.push(p.clone());
        }
    }
}
// FontTexture.add nudges UVs inside the glyph to avoid neighboring texels.
fn glyph_rect(x: u32, y: u32, w: u32, h: u32, atlas: (u32, u32)) -> Rect {
    Rect {
        u0: (x as f32 + 0.01) / atlas.0 as f32,
        v0: (y as f32 + 0.01) / atlas.1 as f32,
        u1: ((x + w) as f32 - 0.01) / atlas.0 as f32,
        v1: ((y + h) as f32 - 0.01) / atlas.1 as f32,
        w,
        h,
    }
}
/// The unscaled pixel width of a bitmap cell (`BitmapProvider.getActualGlyphWidth`).
fn used_width(pixels: &[u8], stride: u32, (cx, cy, cw, ch): (u32, u32, u32, u32)) -> u32 {
    (0..cw)
        .rev()
        .find(|&x| {
            (0..ch).any(|y| {
                pixels
                    .get((((cy + y) * stride + cx + x) * 4 + 3) as usize)
                    .is_some_and(|a| *a != 0)
            })
        })
        .map_or(0, |x| x + 1)
}
/// `FontSet.hasFishyAdvance`: an advance under 0 or over 32. Such a glyph is still the one drawn
/// (`SelectedGlyphs.any` is the first provider's); only `SelectedGlyphs.nonFishy`, what obfuscated
/// text draws, moves on to the next provider that has the character. Packs rely on it: a `space`
/// provider's kerning runs from -512 to 512, and a tall bitmap's glyphs pass 32. Horizon keeps one
/// advance per glyph and tests it here.
fn fishy_advance(advance: f32) -> bool {
    !(0.0..=32.0).contains(&advance)
}
/// A Unihex glyph's advance (`UnihexProvider.Glyph.info().getAdvance`): half its width
/// plus one, the width being the bits from its left to its right column.
fn hex_advance(h: &HexGlyph) -> f32 {
    let width = h.right.saturating_sub(h.left) + 1;
    (width / 2 + 1) as f32
}
/// `BitmapProvider.Definition.load`: each character's glyph, from the atlas
/// when the sheet is in it (`pixels` its RGBA), else from the sheet's own
/// pixels, copied into the atlas when first drawn.
fn load_bitmap(
    source: &ResourceSource,
    p: &serde_json::Value,
    sheets: &HashMap<String, Rect>,
    pixels: &[u8],
    atlas: (u32, u32),
    metrics_only: bool,
) -> crate::fast_hash::FxHashMap<char, BitmapGlyph> {
    let mut out = crate::fast_hash::FxHashMap::default();
    let file = p["file"].as_str().unwrap_or("");
    let (ns, name) = file.split_once(':').unwrap_or(("minecraft", file));
    let name = name.trim_end_matches(".png");
    // The atlas names the default namespace's textures by their path and every
    // other namespace's as `namespace:path` (HudAtlas::build).
    let atlas_name = if ns == "minecraft" {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(format!("{ns}:{name}"))
    };
    // Checked with the whole file (`font_file_error`): rows of one length, none empty.
    let rows: Vec<Vec<char>> = p["chars"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r.as_str().map(|s| s.chars().collect()))
        .collect();
    let Some(width) = rows.first().map(Vec::len).filter(|w| *w > 0) else {
        return out;
    };
    let in_atlas = sheets.get(atlas_name.as_ref()).copied();
    // The sheet: its size, and its pixels when they are not in the atlas.
    let (sw, sh, sheet) = match in_atlas {
        Some(rect) => {
            let (w, h) = rect.texels(atlas);
            (w, h, None)
        }
        None => match source
            .read(&resource_path(file, "textures", ""))
            .map_err(Into::into)
            .and_then(|bytes| crate::hud::decode_bytes(&bytes))
        {
            Ok((w, h, rgba)) => (w, h, Some((w, rgba))),
            Err(_) => {
                log::warn!("font sheet {file} could not be read");
                return out;
            }
        },
    };
    let cell_w = sw / width as u32;
    let cell_h = sh / rows.len() as u32;
    // Checked with the whole file (`font_file_error`).
    let (height, ascent) = bitmap_height_ascent(p).unwrap_or((8, 7));
    let (height, ascent) = (height as f32, ascent as f32);
    let top = 7.0 - ascent;
    // A grid with more rows or columns than the sheet has pixels: each
    // character is an empty glyph of advance 1, as vanilla's arithmetic ends
    // there, and still kept from later providers.
    if cell_w == 0 || cell_h == 0 {
        for ch in rows.iter().flatten().filter(|c| **c != '\0') {
            out.entry(*ch).or_insert(BitmapGlyph::Atlas(Glyph {
                rect: None,
                left: 0.0,
                top,
                right: 0.0,
                bottom: top,
                advance: 1.0,
                offset: 1.0,
            }));
        }
        return out;
    }
    let scale = height / cell_h as f32;
    let origin = in_atlas.map_or((0, 0), |rect| {
        (
            (rect.u0 * atlas.0 as f32).round() as u32,
            (rect.v0 * atlas.1 as f32).round() as u32,
        )
    });
    for (r, row) in rows.iter().enumerate() {
        let mut row_strip: Option<Arc<SheetStrip>> = None;
        for (c, ch) in row.iter().enumerate() {
            // Character 0 pads the grid. A character listed twice keeps its
            // last cell, as vanilla's `Map.put` does.
            if *ch == '\0' {
                continue;
            }
            let (x, y) = (c as u32 * cell_w, r as u32 * cell_h);
            let used = match &sheet {
                Some((stride, rgba)) => used_width(rgba, *stride, (x, y, cell_w, cell_h)),
                None => used_width(
                    pixels,
                    atlas.0,
                    (origin.0 + x, origin.1 + y, cell_w, cell_h),
                ),
            };
            let glyph = Glyph {
                rect: None,
                left: 0.0,
                top,
                right: cell_w as f32 * scale,
                bottom: top + cell_h as f32 * scale,
                advance: (0.5 + used as f32 * scale).floor() + 1.0,
                offset: 1.0,
            };
            if metrics_only {
                out.insert(*ch, BitmapGlyph::Atlas(glyph));
                continue;
            }
            let glyph = match &sheet {
                None => BitmapGlyph::Atlas(Glyph {
                    rect: Some(glyph_rect(
                        origin.0 + x,
                        origin.1 + y,
                        cell_w,
                        cell_h,
                        atlas,
                    )),
                    ..glyph
                }),
                Some((stride, rgba)) => {
                    let strip = row_strip.get_or_insert_with(|| {
                        let start = (y * *stride * 4) as usize;
                        let end = ((y + cell_h) * *stride * 4) as usize;
                        Arc::new(SheetStrip::new(*stride, &rgba[start..end]))
                    });
                    BitmapGlyph::Sheet(SheetCell {
                        strip: Arc::clone(strip),
                        x,
                        w: cell_w,
                        h: cell_h,
                        glyph,
                    })
                }
            };
            out.insert(*ch, glyph);
        }
    }
    out.shrink_to_fit();
    out
}
/// `TrueTypeGlyphProviderDefinition.load`: a TrueType file (not CFF) under
/// `font/`, its Unicode characters but the `skip` ones.
fn load_true_type(source: &ResourceSource, p: &serde_json::Value) -> Option<TrueType> {
    let file = p["file"].as_str()?;
    let data = source
        .read(&resource_path(file, "font", ""))
        .map_err(|e| log::warn!("font file {file}: {e}"))
        .ok()?;
    // FT_Get_Font_Format must say TrueType: CFF outlines ("OTTO") are refused.
    if data.starts_with(b"OTTO") {
        log::warn!("font file {file} is not in TTF format");
        return None;
    }
    let font = ab_glyph::FontVec::try_from_vec(data)
        .map_err(|e| log::warn!("font file {file}: {e}"))
        .ok()?;
    let size = p["size"].as_f64().unwrap_or(11.0) as f32;
    let oversample = p["oversample"].as_f64().unwrap_or(1.0) as f32;
    let shift = p["shift"].as_array().map_or((0.0, 0.0), |s| {
        let f = |i: usize| s.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
        (f(0), f(1))
    });
    let mut skip: Vec<char> = match &p["skip"] {
        serde_json::Value::String(s) => s.chars().collect(),
        serde_json::Value::Array(a) => a
            .iter()
            .filter_map(|v| v.as_str())
            .flat_map(str::chars)
            .collect(),
        _ => Vec::new(),
    };
    skip.sort_unstable();
    skip.dedup();
    skip.shrink_to_fit();
    let pixels_per_em = (size * oversample).round();
    // ab_glyph scales by the font's height; FreeType by its em.
    let units = font.units_per_em().unwrap_or(1000.0);
    let scale = ab_glyph::PxScale::from(pixels_per_em * font.height_unscaled() / units);
    Some(TrueType {
        font,
        skip,
        scale,
        oversample,
        shift: (shift.0 * oversample, shift.1 * oversample),
    })
}
impl TrueType {
    #[inline]
    fn glyph_id(&self, c: char) -> Option<ab_glyph::GlyphId> {
        if !self.skip.is_empty() && self.skip.binary_search(&c).is_ok() {
            return None;
        }
        let id = self.font.glyph_id(c);
        (id.0 != 0).then_some(id)
    }
    /// The advance in GUI pixels: hinted FreeType advances are whole
    /// oversampled pixels.
    fn advance(&self, id: ab_glyph::GlyphId) -> f32 {
        self.font.as_scaled(self.scale).h_advance(id).round() / self.oversample
    }
    /// `loadGlyph` and its bitmap: the grey coverage as `rrrr` texels
    /// (`text_intensity`), below 0.1 cut (its discard).
    fn glyph(&self, id: ab_glyph::GlyphId) -> (Glyph, Option<(u32, u32, Vec<u8>)>) {
        let advance = self.advance(id);
        let glyph =
            id.with_scale_and_position(self.scale, ab_glyph::point(self.shift.0, self.shift.1));
        let empty = Glyph {
            rect: None,
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            advance,
            offset: 1.0,
        };
        let Some(outline) = self.font.outline_glyph(glyph) else {
            return (empty, None);
        };
        let bounds = outline.px_bounds();
        let (w, h) = (bounds.width() as u32, bounds.height() as u32);
        if w == 0 || h == 0 {
            return (empty, None);
        }
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        outline.draw(|x, y, coverage| {
            let v = if coverage < 0.1 {
                0
            } else {
                (coverage.min(1.0) * 255.0).round() as u8
            };
            let i = ((y * w + x) * 4) as usize;
            if let Some(px) = rgba.get_mut(i..i + 4) {
                px.copy_from_slice(&[v; 4]);
            }
        });
        let os = self.oversample;
        // GlyphBitmap: bitmap_left / bitmap_top over the oversampling, and
        // top = 7 - bearingTop.
        let (left, top) = (bounds.min.x / os, 7.0 + bounds.min.y / os);
        (
            Glyph {
                rect: None,
                left,
                top,
                right: left + w as f32 / os,
                bottom: top + h as f32 / os,
                advance,
                offset: 1.0,
            },
            Some((w, h, rgba)),
        )
    }
}
fn load_hex(source: &ResourceSource, p: &serde_json::Value) -> HexTable {
    let mut entries = Vec::new();
    let mut rows = Vec::new();
    let mut by_cp = crate::fast_hash::FxHashMap::default();
    let Some(file) = p["hex_file"].as_str() else {
        return HexTable { entries, rows };
    };
    let path = resource_path(file, "", "").replace("//", "/");
    let Ok(zip) = source.read(&path).and_then(Zip::from_bytes) else {
        return HexTable { entries, rows };
    };
    let names: Vec<String> = zip
        .names()
        .filter(|n| n.ends_with(".hex"))
        .map(str::to_owned)
        .collect();
    for name in names {
        if let Ok(bytes) = zip.read(&name) {
            parse_hex_into(&bytes, &mut entries, &mut rows, &mut by_cp);
        }
    }
    for o in p["size_overrides"].as_array().into_iter().flatten() {
        let bound = |k: &str| {
            o[k].as_str()
                .and_then(|s| s.chars().next())
                .map_or(0, u32::from)
        };
        let left = o["left"].as_u64().unwrap_or(0).min(u64::from(u8::MAX)) as u8;
        let right = o["right"].as_u64().unwrap_or(15).min(u64::from(u8::MAX)) as u8;
        for c in bound("from")..=bound("to") {
            if let Some(&idx) = by_cp.get(&c) {
                entries[idx].left = left;
                entries[idx].right = right;
            }
        }
    }
    drop(by_cp);
    entries.sort_unstable_by_key(|e| e.codepoint);
    entries.shrink_to_fit();
    rows.shrink_to_fit();
    HexTable { entries, rows }
}
fn ensure_unihex<'a>(
    state: &'a mut UnihexState,
    unihex_cache: &mut HashMap<String, Arc<HexTable>>,
) -> &'a HexTable {
    if let UnihexState::Unloaded { source, def } = state {
        let key = def.to_string();
        let table = unihex_cache
            .entry(key)
            .or_insert_with(|| Arc::new(load_hex(source, def)))
            .clone();
        *state = UnihexState::Loaded(table);
    }
    match state {
        UnihexState::Loaded(table) => table,
        UnihexState::Unloaded { .. } => unreachable!(),
    }
}
/// The ids of every font definition (`font/**/*.json` of each namespace).
fn font_ids(source: &ResourceSource) -> Vec<String> {
    source
        .list_under("assets", &["font"])
        .into_iter()
        .filter_map(|file| {
            let (namespace, path) = file.strip_prefix("assets/")?.split_once("/font/")?;
            Some(format!("{namespace}:{}", path.strip_suffix(".json")?))
        })
        .collect()
}

impl Region {
    /// Place `w x h` pixels in the region (rows left to right); their atlas
    /// position, or none when it is full.
    fn place(
        &mut self,
        w: u32,
        h: u32,
        rgba: Vec<u8>,
        atlas: (u32, u32),
        region: (u32, u32),
    ) -> Option<(u32, u32)> {
        let (mut x, mut y) = self.cursor;
        if x + w + 1 > atlas.0 {
            x = 0;
            y += self.row + 1;
            self.row = 0;
        }
        if y + h > region.1 {
            self.full = true;
            return None;
        }
        let (ax, ay) = (x, region.0 + y);
        self.pending.push(Patch {
            x: ax,
            y: ay,
            width: w,
            height: h,
            rgba,
        });
        self.cursor = (x + w + 1, y);
        self.row = self.row.max(h);
        Some((ax, ay))
    }
}
/// A value no font has had yet (`Font::epoch`).
fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
impl Font {
    /// Build from the atlas holding the bitmap font sheets (`sprites`, of
    /// `atlas` size) with `region` the reserved rows.
    #[horizon_mod_api_macros::replaceable]
    pub fn load(
        source: &ResourceSource,
        sprites: &HashMap<String, Rect>,
        pixels: &[u8],
        atlas: (u32, u32),
        region: (u32, u32),
        white: Rect,
    ) -> Self {
        Self::load_with(source, sprites, pixels, atlas, region, white, false)
    }
    /// Resolve provider files against the game's resources when an overlay lacks them.
    /// Definitions remain the overlay's so pack filters still hide lower definitions.
    #[allow(clippy::too_many_arguments)]
    pub fn load_over(
        source: &ResourceSource,
        defaults: &ResourceSource,
        sprites: &HashMap<String, Rect>,
        pixels: &[u8],
        atlas: (u32, u32),
        region: (u32, u32),
        white: Rect,
    ) -> Self {
        if source == defaults {
            return Self::load(source, sprites, pixels, atlas, region, white);
        }
        use horizon_assets::resource_source::Resource;
        // Fonts only ever read definitions/TTFs/zips under `/font/`, off-atlas bitmap
        // sheets under `/textures/`, or external `.zip` unihex archives. Skip sounds,
        // models, blockstates, lang files, etc. so the temporary index stays small.
        let font_file =
            |p: &str| p.contains("/font/") || p.contains("/textures/") || p.ends_with(".zip");
        let mut files: std::collections::BTreeMap<_, _> = defaults
            .list("assets/")
            .into_iter()
            .filter(|p| font_file(p) && !(p.contains("/font/") && p.ends_with(".json")))
            .map(|p| (p.clone(), Resource::file(defaults.clone(), p)))
            .collect();
        files.extend(
            source
                .list("assets/")
                .into_iter()
                .filter(|p| font_file(p))
                .map(|p| (p.clone(), Resource::file(source.clone(), p))),
        );
        Self::load(
            &ResourceSource::indexed(files),
            sprites,
            pixels,
            atlas,
            region,
            white,
        )
    }
    /// A server measures text but never draws it. Decode bitmap sheets only
    /// to obtain advances, then release their pixels; never rasterize glyphs.
    pub fn metrics(source: &ResourceSource) -> Self {
        let white = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 1,
            h: 1,
        };
        Self::load_with(source, &HashMap::new(), &[], (1, 1), (0, 0), white, true)
    }
    #[allow(clippy::too_many_arguments)]
    fn load_with(
        source: &ResourceSource,
        sprites: &HashMap<String, Rect>,
        pixels: &[u8],
        atlas: (u32, u32),
        region: (u32, u32),
        white: Rect,
        metrics_only: bool,
    ) -> Self {
        let mut font = Self {
            missing: Glyph {
                rect: None,
                left: 0.0,
                top: 0.0,
                right: 5.0,
                bottom: 8.0,
                advance: 6.0,
                offset: 1.0,
            },
            white,
            atlas,
            region,
            metrics_only,
            default_set: None,
            cache: RefCell::new(Cache {
                sets: Vec::new(),
                region: Region {
                    cursor: (0, 0),
                    row: 0,
                    pending: Vec::new(),
                    full: false,
                },
                unihex: HashMap::new(),
                // `Font.random`: `RandomSource.create()`.
                random: crate::random_source::create(),
            }),
            generation: std::cell::Cell::new(next_generation()),
            uniform: false,
            japanese: false,
        };
        // FontManager.prepare: every font definition of the resources.
        let mut ids = font_ids(source);
        ids.sort_by_key(|id| id != DEFAULT);
        let sets: Vec<(String, FontSet)> = ids
            .into_iter()
            .map(|id| {
                let set = font.load_set(source, &id, sprites, pixels);
                (id, set)
            })
            .collect();
        font.default_set = sets.iter().position(|(id, _)| id == DEFAULT);
        font.cache.borrow_mut().sets = sets;
        font
    }
    /// `FontManager`: a font's providers with references expanded.
    fn load_set(
        &self,
        source: &ResourceSource,
        id: &str,
        sprites: &HashMap<String, Rect>,
        pixels: &[u8],
    ) -> FontSet {
        let mut all = Vec::new();
        providers(source, id, 0, &mut all);
        let providers = all
            .iter()
            .filter_map(|p| match p["type"].as_str()? {
                "space" => {
                    let mut advances: crate::fast_hash::FxHashMap<char, f32> = p["advances"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter_map(|(k, v)| Some((k.chars().next()?, v.as_f64()? as f32)))
                        .collect();
                    advances.shrink_to_fit();
                    Some(Provider::Space(advances))
                }
                "bitmap" => Some(Provider::Bitmap(
                    Filter::of(p),
                    load_bitmap(source, p, sprites, pixels, self.atlas, self.metrics_only),
                )),
                "unihex" => Some(Provider::Unihex(
                    Filter::of(p),
                    UnihexState::Unloaded {
                        source: source.clone(),
                        def: p.clone(),
                    },
                )),
                "ttf" => Some(Provider::TrueType(
                    Filter::of(p),
                    Box::new(load_true_type(source, p)?),
                )),
                _ => None,
            })
            .collect();
        FontSet {
            providers,
            ascii: None,
            glyphs: Default::default(),
            non_fishy: Default::default(),
            by_width: None,
        }
    }
    /// Forget rasterized glyphs (after changing font options).
    pub fn reset_cache(&self) {
        let mut cache = self.cache.borrow_mut();
        for (_, set) in &mut cache.sets {
            set.clear_cache();
        }
        cache.region.cursor = (0, 0);
        cache.region.row = 0;
        cache.region.full = false;
        self.generation.set(next_generation());
    }
    /// Call once per frame before any text is laid out. When the dynamic
    /// region ran out of room (vanilla's glyph textures grow instead), the
    /// glyphs are forgotten so they are rasterized again from the start; it
    /// waits for the frame boundary because quads already queued hold the old
    /// rects.
    pub fn begin_frame(&self) {
        if self.cache.borrow().region.full {
            self.reset_cache();
        }
    }
    /// Changes whenever a glyph may come out different: another font, a
    /// cache reset (glyphs move in the atlas) or other font options. Text laid
    /// out under one epoch can be kept until it changes.
    pub fn epoch(&self) -> u64 {
        self.generation.get() << 2 | u64::from(self.uniform) << 1 | u64::from(self.japanese)
    }
    #[inline]
    fn set_index(&self, cache: &Cache, id: &str) -> Option<usize> {
        if id == DEFAULT {
            return self.default_set;
        }
        cache.sets.iter().position(|(k, _)| k == id)
    }
    #[inline]
    fn style_set_index(&self, cache: &Cache, style: &Style) -> Option<usize> {
        match style.font.as_deref() {
            None => self.default_set,
            Some(id) => self.set_index(cache, id),
        }
    }
    /// The glyph for a character in the default font (FontSet.getGlyph).
    pub fn glyph(&self, c: char) -> Glyph {
        let Some(idx) = self.default_set else {
            return self.missing;
        };
        let mut cache = self.cache.borrow_mut();
        self.glyph_at(&mut cache, idx, c)
    }
    /// The glyph for a character drawn in `style`'s font (its id is
    /// already complete, see `text::font_id`).
    pub fn glyph_styled(&self, c: char, style: &Style) -> Glyph {
        let mut cache = self.cache.borrow_mut();
        let Some(idx) = self.style_set_index(&cache, style) else {
            return self.missing;
        };
        self.glyph_at(&mut cache, idx, c)
    }
    /// The glyph for a character in the font `id` (`alt` is `minecraft:alt`).
    pub fn glyph_in(&self, id: &str, c: char) -> Glyph {
        let mut cache = self.cache.borrow_mut();
        let idx = if id.contains(':') {
            self.set_index(&cache, id)
        } else {
            cache
                .sets
                .iter()
                .position(|(k, _)| k.strip_prefix("minecraft:") == Some(id))
        };
        let Some(idx) = idx else {
            return self.missing;
        };
        self.glyph_at(&mut cache, idx, c)
    }
    /// The first active provider's glyph in `sets[set_idx]`, else the missing glyph.
    #[inline]
    fn glyph_at(&self, cache: &mut Cache, set_idx: usize, c: char) -> Glyph {
        let Cache {
            sets,
            region,
            unihex,
            ..
        } = cache;
        let set = &mut sets[set_idx].1;
        if let Some(g) = set.get_glyph(c) {
            return g;
        }
        let was_full = std::mem::replace(&mut region.full, false);
        let found = self.resolve(set, region, unihex, c, false);
        // No room: the missing glyph for now, not remembered, so the
        // character is tried again once `begin_frame` cleared the region.
        let overflowed = std::mem::replace(&mut region.full, was_full);
        region.full |= overflowed;
        if overflowed {
            return self.missing;
        }
        let glyph = found.unwrap_or(self.missing);
        set.insert_glyph(c, glyph);
        glyph
    }
    #[inline]
    fn advance_in(
        &self,
        cache: &mut Cache,
        set_idx: Option<usize>,
        c: char,
        bold: bool,
        has_inline: bool,
    ) -> f32 {
        if c == '\u{fffc}' && has_inline {
            return 8.0;
        }
        let Some(idx) = set_idx else {
            return self.missing.advance + if bold { self.missing.offset } else { 0.0 };
        };
        if !bold
            && (c as u32) < 128
            && let Some(ascii) = cache.sets[idx].1.ascii.as_deref()
        {
            let adv = ascii.advances[c as usize];
            if !adv.is_nan() {
                return adv;
            }
        }
        let g = self.glyph_at(cache, idx, c);
        g.advance + if bold { g.offset } else { 0.0 }
    }
    #[inline]
    fn ascii_advance_at(&self, cache: &mut Cache, set_idx: usize, byte: u8) -> f32 {
        if let Some(ascii) = cache.sets[set_idx].1.ascii.as_deref() {
            let adv = ascii.advances[byte as usize];
            if !adv.is_nan() {
                return adv;
            }
        }
        self.glyph_at(cache, set_idx, char::from(byte)).advance
    }
    /// `FontSet.getRandomGlyph`'s `SelectedGlyphs.nonFishy`: the glyph of the first provider whose
    /// advance is not fishy, else the missing glyph. A character whose first glyph is fine has
    /// the same one for both.
    #[cfg(test)]
    fn glyph_non_fishy(&self, id: &str, c: char) -> Glyph {
        let mut cache = self.cache.borrow_mut();
        let Some(idx) = self.set_index(&cache, id) else {
            return self.missing;
        };
        self.glyph_non_fishy_at(&mut cache, idx, c)
    }
    fn glyph_non_fishy_at(&self, cache: &mut Cache, set_idx: usize, c: char) -> Glyph {
        let any = self.glyph_at(cache, set_idx, c);
        if !fishy_advance(any.advance) {
            return any;
        }
        let Cache {
            sets,
            region,
            unihex,
            ..
        } = cache;
        let set = &mut sets[set_idx].1;
        if let Some(g) = set.non_fishy.get(&c) {
            return *g;
        }
        let was_full = std::mem::replace(&mut region.full, false);
        let found = self.resolve(set, region, unihex, c, true);
        let overflowed = std::mem::replace(&mut region.full, was_full);
        region.full |= overflowed;
        if overflowed {
            return self.missing;
        }
        let glyph = found.unwrap_or(self.missing);
        set.non_fishy.insert(c, glyph);
        glyph
    }
    fn resolve(
        &self,
        set: &mut FontSet,
        region: &mut Region,
        unihex_cache: &mut HashMap<String, Arc<HexTable>>,
        c: char,
        non_fishy: bool,
    ) -> Option<Glyph> {
        let (uniform, japanese) = (self.uniform, self.japanese);
        for provider in &mut set.providers {
            match provider {
                Provider::Space(advances) => {
                    if let Some(a) = advances.get(&c)
                        && !(non_fishy && fishy_advance(*a))
                    {
                        return Some(Glyph {
                            rect: None,
                            left: 0.0,
                            top: 0.0,
                            right: 0.0,
                            bottom: 0.0,
                            advance: *a,
                            offset: 1.0,
                        });
                    }
                }
                Provider::Bitmap(filter, glyphs) if filter.passes(uniform, japanese) => {
                    match glyphs.get(&c) {
                        Some(BitmapGlyph::Atlas(g)) if !(non_fishy && fishy_advance(g.advance)) => {
                            return Some(*g);
                        }
                        Some(BitmapGlyph::Sheet(cell))
                            if !(non_fishy && fishy_advance(cell.glyph.advance)) =>
                        {
                            let Some(rgba) = cell.pixels() else {
                                continue;
                            };
                            let (x, y) =
                                region.place(cell.w, cell.h, rgba, self.atlas, self.region)?;
                            return Some(Glyph {
                                rect: Some(glyph_rect(x, y, cell.w, cell.h, self.atlas)),
                                ..cell.glyph
                            });
                        }
                        _ => {}
                    }
                }
                Provider::Unihex(filter, state) if filter.passes(uniform, japanese) => {
                    let table = ensure_unihex(state, unihex_cache);
                    if let Some(h) = table.get(u32::from(c))
                        && !(non_fishy && fishy_advance(hex_advance(&h)))
                    {
                        return self.rasterize_hex(region, h);
                    }
                }
                Provider::TrueType(filter, ttf) if filter.passes(uniform, japanese) => {
                    if let Some(id) = ttf.glyph_id(c)
                        && !(non_fishy && fishy_advance(ttf.advance(id)))
                    {
                        if self.metrics_only {
                            return Some(Glyph {
                                rect: None,
                                advance: ttf.advance(id),
                                offset: 1.0,
                                ..self.missing
                            });
                        }
                        let (glyph, bitmap) = ttf.glyph(id);
                        return match bitmap {
                            Some((w, h, rgba)) => {
                                let (x, y) = region.place(w, h, rgba, self.atlas, self.region)?;
                                Some(Glyph {
                                    rect: Some(glyph_rect(x, y, w, h, self.atlas)),
                                    ..glyph
                                })
                            }
                            None => Some(glyph),
                        };
                    }
                }
                _ => {}
            }
        }
        None
    }
    fn rasterize_hex(&self, region: &mut Region, h: HexGlyph) -> Option<Glyph> {
        let width = h.right.saturating_sub(h.left) + 1;
        if self.metrics_only {
            return Some(Glyph {
                rect: None,
                left: 0.0,
                top: 0.0,
                right: width as f32 / 2.0,
                bottom: 8.0,
                advance: hex_advance(&h),
                offset: 0.5,
            });
        }
        let mut rgba = Vec::with_capacity((width * 16 * 4) as usize);
        for row in h.rows {
            for bit in h.left..=h.right {
                let set = bit < 32 && (row >> (31 - bit)) & 1 != 0;
                rgba.extend(if set { [255; 4] } else { [0; 4] });
            }
        }
        let (x, y) = region.place(width, 16, rgba, self.atlas, self.region)?;
        Some(Glyph {
            rect: Some(glyph_rect(x, y, width, 16, self.atlas)),
            left: 0.0,
            top: 0.0,
            right: width as f32 / 2.0,
            bottom: 8.0,
            advance: hex_advance(&h),
            offset: 0.5,
        })
    }
    /// Starts the random `random_glyph` draws on over from `seed`.
    pub fn reseed(&self, seed: i64) {
        self.cache.borrow_mut().random = horizon_core::random::LegacyRandom::new(seed);
    }
    /// `FontSet.getRandomGlyph`: what an obfuscated character draws, a random
    /// glyph of the font as wide as `advance` (rounded up), else the missing
    /// glyph.
    ///
    /// Vanilla picks among every glyph of the font set, Unifont's included.
    /// Those are rasterized into a fixed atlas region on demand, which a
    /// line of obfuscated text would fill in seconds, so the candidates are
    /// the glyphs already in the atlas (the bitmap sheets there and spaces),
    /// or for a font without any (Force Unicode Font, a TrueType font) its
    /// glyphs below U+0250.
    pub fn random_glyph(&self, style: &Style, advance: f32) -> Glyph {
        use horizon_core::random::RandomSource;
        let width = advance.ceil() as i32;
        let mut cache = self.cache.borrow_mut();
        let Some(set_idx) = self.style_set_index(&cache, style) else {
            return self.missing;
        };
        let pick = {
            let Cache {
                sets,
                unihex,
                random,
                ..
            } = &mut *cache;
            let set = &mut sets[set_idx].1;
            if set.by_width.is_none() {
                set.by_width = Some(self.glyphs_by_width(set, unihex));
            }
            set.by_width
                .as_ref()
                .and_then(|m| m.get(&width))
                .and_then(|chars| {
                    let i = random.next_int_bound(chars.len() as i32).ok()?;
                    chars.get(i as usize).copied()
                })
        };
        pick.map_or(self.missing, |c| {
            self.glyph_non_fishy_at(&mut cache, set_idx, c)
        })
    }
    fn glyphs_by_width(
        &self,
        set: &mut FontSet,
        unihex_cache: &mut HashMap<String, Arc<HexTable>>,
    ) -> crate::fast_hash::FxHashMap<i32, Vec<char>> {
        let (uniform, japanese) = (self.uniform, self.japanese);
        let mut chars = std::collections::BTreeSet::new();
        let mut in_atlas = false;
        for provider in &set.providers {
            match provider {
                Provider::Space(advances) => chars.extend(advances.keys().copied()),
                Provider::Bitmap(filter, glyphs) if filter.passes(uniform, japanese) => {
                    for (c, g) in glyphs {
                        if matches!(g, BitmapGlyph::Atlas(_)) {
                            in_atlas = true;
                            chars.insert(*c);
                        }
                    }
                }
                _ => {}
            }
        }
        if !in_atlas {
            // Nothing in the atlas: the glyphs below U+0250, rasterized as used.
            chars.extend((0..0x250u32).filter_map(char::from_u32));
        }
        let mut out: crate::fast_hash::FxHashMap<i32, Vec<char>> =
            crate::fast_hash::FxHashMap::default();
        for c in chars {
            if let Some(advance) = self.provider_advance(set, unihex_cache, c) {
                out.entry(advance.ceil() as i32).or_default().push(c);
            }
        }
        out
    }
    /// A character's advance in `set` (its first provider that has it, fishy or not, as the
    /// by-width table of `FontSet.reload` takes it), without rasterizing it.
    fn provider_advance(
        &self,
        set: &mut FontSet,
        unihex_cache: &mut HashMap<String, Arc<HexTable>>,
        c: char,
    ) -> Option<f32> {
        let (uniform, japanese) = (self.uniform, self.japanese);
        for provider in &mut set.providers {
            match provider {
                Provider::Space(advances) => {
                    if let Some(a) = advances.get(&c) {
                        return Some(*a);
                    }
                }
                Provider::Bitmap(filter, glyphs) if filter.passes(uniform, japanese) => {
                    match glyphs.get(&c) {
                        Some(BitmapGlyph::Atlas(g)) => {
                            return Some(g.advance);
                        }
                        Some(BitmapGlyph::Sheet(cell)) => {
                            return Some(cell.glyph.advance);
                        }
                        _ => {}
                    }
                }
                Provider::Unihex(filter, state) if filter.passes(uniform, japanese) => {
                    let table = ensure_unihex(state, unihex_cache);
                    if let Some(adv) = table.advance(u32::from(c)) {
                        return Some(adv);
                    }
                }
                Provider::TrueType(filter, ttf) if filter.passes(uniform, japanese) => {
                    if let Some(id) = ttf.glyph_id(c) {
                        return Some(ttf.advance(id));
                    }
                }
                _ => {}
            }
        }
        None
    }
    /// Atlas pixels rasterized since the last call.
    #[horizon_mod_api_macros::replaceable]
    pub fn take_patches(&self) -> Vec<Patch> {
        std::mem::take(&mut self.cache.borrow_mut().region.pending)
    }
    pub fn advance(&self, c: char, style: &Style) -> f32 {
        if c == '\u{fffc}' && style.inline_object.is_some() {
            return 8.0;
        }
        let mut cache = self.cache.borrow_mut();
        let set_idx = self.style_set_index(&cache, style);
        self.advance_in(&mut cache, set_idx, c, style.is_bold(), false)
    }
    /// Font.width of plain text (legacy codes are formatting, not glyphs).
    pub fn width(&self, s: &str) -> f32 {
        if s.is_empty() {
            return 0.0;
        }
        let rtl = crate::lang::bidirectional();
        if !rtl && s.is_ascii() {
            let Some(idx) = self.default_set else {
                return (self.missing.advance * s.len() as f32).ceil();
            };
            let mut cache = self.cache.borrow_mut();
            let mut total = 0.0f32;
            for b in s.bytes() {
                total += self.ascii_advance_at(&mut cache, idx, b);
            }
            return total.ceil();
        }
        if s.contains('§') {
            if !rtl && !s.chars().any(|c| c as u32 >= 0x0590) {
                let mut cache = self.cache.borrow_mut();
                let set_idx = self.default_set;
                let mut total = 0.0f32;
                let mut bold = false;
                let mut chars = s.chars();
                while let Some(c) = chars.next() {
                    if c == '§' {
                        let Some(code) = chars.next() else { break };
                        match code.to_ascii_lowercase() {
                            '0'..='9' | 'a'..='f' | 'r' => bold = false,
                            'l' => bold = true,
                            _ => {}
                        }
                        continue;
                    }
                    total += self.advance_in(&mut cache, set_idx, c, bold, false);
                }
                return total.ceil();
            }
            return self.width_text(&Text::legacy(s));
        }
        let visual = crate::bidi::visual_str(s);
        let mut cache = self.cache.borrow_mut();
        let set_idx = self.default_set;
        let mut total = 0.0f32;
        for c in visual.chars() {
            total += self.advance_in(&mut cache, set_idx, c, false, false);
        }
        total.ceil()
    }
    fn measure_spans(&self, t: &Text) -> f32 {
        let mut cache = self.cache.borrow_mut();
        let mut total = 0.0f32;
        for span in &t.spans {
            if span.text.is_empty() {
                continue;
            }
            let set_idx = self.style_set_index(&cache, &span.style);
            let bold = span.style.is_bold();
            let has_inline = span.style.inline_object.is_some();
            if !bold
                && !has_inline
                && span.text.is_ascii()
                && let Some(idx) = set_idx
            {
                for b in span.text.bytes() {
                    total += self.ascii_advance_at(&mut cache, idx, b);
                }
            } else {
                for c in span.text.chars() {
                    total += self.advance_in(&mut cache, set_idx, c, bold, has_inline);
                }
            }
        }
        total.ceil()
    }
    /// Font.width of a component: `§` codes in the spans are formatting,
    /// not glyphs. Right-to-left text is measured as it is drawn, shaped.
    pub fn width_text(&self, t: &Text) -> f32 {
        if !crate::lang::bidirectional() && t.spans.iter().all(|s| s.text.is_ascii()) {
            return self.measure_spans(t);
        }
        let t = t.resolved();
        let visual = crate::bidi::visual(&t);
        self.measure_spans(&visual)
    }
    /// `Font.width(FormattedCharSequence)` of a sequence a component built
    /// with `FormattedCharSequence.forward`: every character is a glyph, a
    /// `§` included (`StringDecomposer.iterate`, not `iterateFormatted`).
    pub fn width_drawn(&self, t: &Text) -> f32 {
        if !crate::lang::bidirectional() && t.spans.iter().all(|s| s.text.is_ascii()) {
            return self.measure_spans(t);
        }
        let visual = crate::bidi::visual(t);
        self.measure_spans(&visual)
    }
    /// `StringSplitter.splitLines`: each line and whether it continues a
    /// wrapped one. A line breaks at a `\n`, or once a character overflows
    /// `max_width` (never the first visible one) at the last space so far, or
    /// before that character; the `\n` or space at the break is dropped.
    pub fn split_lines(&self, t: &Text, max_width: f32) -> Vec<(Text, bool)> {
        let max_width = max_width.max(1.0);
        let spans = &t.spans;
        let end_pos = SpanPos {
            span: spans.len(),
            byte: 0,
        };
        let mut out = Vec::new();
        let mut start = next_non_empty_pos(spans, SpanPos { span: 0, byte: 0 });
        let (mut wrapped, mut force_new_line) = (false, false);
        let mut cache = self.cache.borrow_mut();
        loop {
            // LineBreakFinder over what is left.
            let (mut width, mut had_visible) = (0.0f32, false);
            let mut last_space: Option<(SpanPos, SpanPos)> = None;
            let mut split: Option<(SpanPos, SpanPos, bool)> = None;
            let (mut si, mut byte) = (start.span, start.byte);
            'scan: while si < spans.len() {
                let span = &spans[si];
                let set_idx = self.style_set_index(&cache, &span.style);
                let bold = span.style.is_bold();
                let has_inline = span.style.inline_object.is_some();
                for (rel, c) in span.text[byte..].char_indices() {
                    let pos = SpanPos {
                        span: si,
                        byte: byte + rel,
                    };
                    let after = SpanPos {
                        span: si,
                        byte: byte + rel + c.len_utf8(),
                    };
                    if c == '\n' {
                        split = Some((pos, after, true));
                        break 'scan;
                    }
                    if c == ' ' {
                        last_space = Some((pos, after));
                    }
                    let advance = self.advance_in(&mut cache, set_idx, c, bold, has_inline);
                    width += advance;
                    if !had_visible || width <= max_width {
                        had_visible |= advance != 0.0;
                    } else {
                        let (at, next) = last_space.unwrap_or((pos, pos));
                        split = Some((at, next, false));
                        break 'scan;
                    }
                }
                si += 1;
                byte = 0;
            }
            let Some((at, next, new_line)) = split else {
                break;
            };
            force_new_line = new_line;
            out.push((slice_spans(spans, start, at), wrapped));
            wrapped = !new_line;
            start = next_non_empty_pos(spans, next);
        }
        if start < end_pos {
            out.push((slice_spans(spans, start, end_pos), wrapped));
        } else if force_new_line {
            out.push((Text::empty(), false));
        }
        out
    }
    /// `Font.split`: `StringSplitter.splitLines(text, width, Style.EMPTY)` over
    /// the text's parts with their `§` codes in. A code styles what follows
    /// (`§r` back to no style while measuring), and the part a line breaks in
    /// goes on in the style at the break. Lines keep their codes for drawing.
    pub fn split_formatted(&self, t: &Text, max_width: f32) -> Vec<Text> {
        let max_width = max_width.max(1.0);
        let mut parts: VecDeque<(&str, &Span, Option<Style>)> = t
            .spans
            .iter()
            .filter(|s| !s.text.is_empty())
            .map(|s| (s.text.as_str(), s, None))
            .collect();
        let mut out = Vec::new();
        let mut force_new_line = false;
        let mut cache = self.cache.borrow_mut();
        // One LineBreakFinder per line, over the parts left.
        while let Some((part_idx, byte_idx, new_line, skip_bytes, split_style)) =
            self.line_break_formatted(&mut cache, &parts, max_width)
        {
            force_new_line = new_line;
            out.push(split_at_formatted(
                &mut parts,
                part_idx,
                byte_idx,
                skip_bytes,
                split_style,
            ));
        }
        if !parts.is_empty() {
            out.push(Text::from_spans(
                parts
                    .into_iter()
                    .map(|(text, orig, style_override)| {
                        let mut span = orig.with_text(text);
                        if let Some(st) = style_override {
                            span.style = st;
                        }
                        span
                    })
                    .collect(),
            ));
        } else if force_new_line {
            out.push(Text::empty());
        }
        out
    }
    /// `StringSplitter.LineBreakFinder` fed by `StringDecomposer.iterateFormatted`:
    /// where the parts break `(part_idx, byte_idx, new_line, skip_bytes, restyle)`.
    fn line_break_formatted(
        &self,
        cache: &mut Cache,
        parts: &VecDeque<(&str, &Span, Option<Style>)>,
        max_width: f32,
    ) -> Option<(usize, usize, bool, usize, Option<Style>)> {
        let (mut width, mut had_visible) = (0.0f32, false);
        let mut last_space: Option<(usize, usize, Option<Style>)> = None;
        for (part_idx, (contents, orig_span, style_override)) in parts.iter().enumerate() {
            let base_style = style_override.as_ref().unwrap_or(&orig_span.style);
            let mut owned_style: Option<Style> = None;
            let mut set_idx = self.style_set_index(cache, base_style);
            let mut bold = base_style.is_bold();
            let mut has_inline = base_style.inline_object.is_some();
            let mut iter = contents.char_indices();
            while let Some((byte_idx, c)) = iter.next() {
                if c == '§' {
                    // A `§` that ends its part ends only that part.
                    let Some((_, code)) = iter.next() else {
                        break;
                    };
                    let cur = owned_style.as_ref().unwrap_or(base_style);
                    let next = legacy_format(cur, code, &Style::default());
                    set_idx = self.style_set_index(cache, &next);
                    bold = next.is_bold();
                    has_inline = next.inline_object.is_some();
                    owned_style = Some(next);
                    continue;
                }
                if c == '\n' {
                    return Some((part_idx, byte_idx, true, 1, owned_style));
                }
                if c == ' ' {
                    last_space = Some((part_idx, byte_idx, owned_style.clone()));
                }
                let advance = self.advance_in(cache, set_idx, c, bold, has_inline);
                width += advance;
                if had_visible && width > max_width {
                    return Some(match last_space {
                        Some((pi, bi, st)) => (pi, bi, false, 1, st),
                        None => (part_idx, byte_idx, false, 0, owned_style),
                    });
                }
                had_visible |= advance != 0.0;
            }
        }
        None
    }
    /// `ComponentRenderUtils.wrapComponents`: the split lines, wrapped ones
    /// indented by an unstyled space; one empty line for empty text.
    pub fn wrap_components(&self, t: &Text, max_width: i32) -> Vec<Text> {
        let lines: Vec<Text> = self
            .split_lines(t, max_width as f32)
            .into_iter()
            .map(|(line, wrapped)| {
                if wrapped {
                    let mut spans = Text::literal(" ").into_spans();
                    spans.extend(line.into_spans());
                    Text::from_spans(spans)
                } else {
                    line
                }
            })
            .collect();
        if lines.is_empty() {
            vec![Text::empty()]
        } else {
            lines
        }
    }
    /// StringSplitter.splitLines: wrap at the last space that fits (or
    /// mid-word when none does); `\n` always breaks. Styles are kept.
    pub fn split(&self, t: &Text, max_width: f32) -> Vec<Text> {
        let spans = &t.spans;
        let end_pos = SpanPos {
            span: spans.len(),
            byte: 0,
        };
        let mut lines = Vec::new();
        let mut start = next_non_empty_pos(spans, SpanPos { span: 0, byte: 0 });
        let mut cache = self.cache.borrow_mut();
        while start < end_pos {
            let mut width = 0.0f32;
            let mut end = end_pos;
            let mut next = end_pos;
            let mut last_space: Option<(SpanPos, SpanPos)> = None;
            let mut ended_with_newline = false;
            let (mut si, mut byte) = (start.span, start.byte);
            'scan: while si < spans.len() {
                let span = &spans[si];
                let set_idx = self.style_set_index(&cache, &span.style);
                let bold = span.style.is_bold();
                let has_inline = span.style.inline_object.is_some();
                for (rel, c) in span.text[byte..].char_indices() {
                    let pos = SpanPos {
                        span: si,
                        byte: byte + rel,
                    };
                    let after = SpanPos {
                        span: si,
                        byte: byte + rel + c.len_utf8(),
                    };
                    if c == '\n' {
                        end = pos;
                        next = after;
                        ended_with_newline = true;
                        break 'scan;
                    }
                    if c == ' ' {
                        last_space = Some((pos, after));
                    }
                    width += self.advance_in(&mut cache, set_idx, c, bold, has_inline);
                    if width > max_width {
                        match last_space {
                            Some((s, s_after)) if s > start => {
                                end = s;
                                next = s_after;
                            }
                            _ => {
                                end = if pos > start { pos } else { after };
                                next = end;
                            }
                        }
                        break 'scan;
                    }
                }
                si += 1;
                byte = 0;
            }
            lines.push(slice_spans(spans, start, end));
            start = next_non_empty_pos(spans, next);
            if start == end_pos && ended_with_newline {
                lines.push(Text::empty());
            }
        }
        if lines.is_empty() {
            lines.push(Text::empty());
        }
        lines
    }
    /// Characters of `s` that fit in `width` (Font.plainSubstrByWidth).
    pub fn fit(&self, s: &str, width: f32) -> usize {
        let mut cache = self.cache.borrow_mut();
        let set_idx = self.default_set;
        let mut w = 0.0f32;
        if s.is_ascii()
            && let Some(idx) = set_idx
        {
            for (i, b) in s.bytes().enumerate() {
                w += self.ascii_advance_at(&mut cache, idx, b);
                if w > width {
                    return i;
                }
            }
            return s.len();
        }
        let mut count = 0usize;
        for (i, c) in s.chars().enumerate() {
            w += self.advance_in(&mut cache, set_idx, c, false, false);
            if w > width {
                return i;
            }
            count = i + 1;
        }
        count
    }
    /// Characters of `s` that fit in `width`, measured from the end.
    pub fn fit_reverse(&self, s: &str, width: f32) -> usize {
        let mut cache = self.cache.borrow_mut();
        let set_idx = self.default_set;
        let mut w = 0.0f32;
        if s.is_ascii()
            && let Some(idx) = set_idx
        {
            for (i, b) in s.bytes().rev().enumerate() {
                w += self.ascii_advance_at(&mut cache, idx, b);
                if w > width {
                    return i;
                }
            }
            return s.len();
        }
        let mut count = 0usize;
        for (i, c) in s.chars().rev().enumerate() {
            w += self.advance_in(&mut cache, set_idx, c, false, false);
            if w > width {
                return i;
            }
            count = i + 1;
        }
        count
    }
    pub fn white(&self) -> Rect {
        self.white
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SpanPos {
    span: usize,
    byte: usize,
}
fn next_non_empty_pos(spans: &[Span], mut pos: SpanPos) -> SpanPos {
    while pos.span < spans.len() && pos.byte >= spans[pos.span].text.len() {
        pos.span += 1;
        pos.byte = 0;
    }
    pos
}
fn slice_spans(spans: &[Span], from: SpanPos, to: SpanPos) -> Text {
    if from >= to {
        return Text::empty();
    }
    let last = to.span.min(spans.len().saturating_sub(1));
    let mut out = Vec::new();
    for si in from.span..=last {
        let Some(span) = spans.get(si) else { break };
        let lo = if si == from.span { from.byte } else { 0 };
        let hi = if si == to.span {
            to.byte
        } else {
            span.text.len()
        };
        if lo < hi {
            out.push(span.with_text(&span.text[lo..hi]));
        }
    }
    Text::from_spans(out)
}
/// `StringSplitter.FlatComponents.splitAt`: the parts before `(part_idx, byte_idx)` as a line,
/// `skip_bytes` dropped, and the part it ends in restyled with `style_override` when set.
fn split_at_formatted(
    parts: &mut VecDeque<(&str, &Span, Option<Style>)>,
    part_idx: usize,
    byte_idx: usize,
    skip_bytes: usize,
    style_override: Option<Style>,
) -> Text {
    let mut line = Vec::new();
    for _ in 0..part_idx {
        if let Some((text, orig, current_style)) = parts.pop_front() {
            let mut span = orig.with_text(text);
            if let Some(st) = current_style {
                span.style = st;
            }
            line.push(span);
        }
    }
    let pop_front = if let Some((text, orig, current_style)) = parts.front_mut() {
        let s = *text;
        if byte_idx > 0 {
            let mut span = orig.with_text(&s[..byte_idx]);
            if let Some(st) = current_style.as_ref() {
                span.style = st.clone();
            }
            line.push(span);
        }
        let rest_start = byte_idx + skip_bytes;
        if rest_start >= s.len() {
            true
        } else {
            *text = &s[rest_start..];
            if style_override.is_some() {
                *current_style = style_override;
            }
            false
        }
    } else {
        false
    };
    if pop_front {
        parts.pop_front();
    }
    Text::from_spans(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    #[test]
    fn unihex_rows_and_widths() {
        let mut out = HashMap::new();
        // 'A' from unifont (8 wide) and a 16-wide glyph with an empty left column.
        let wide = format!("4E00:{}7FFE{}\n", "0000".repeat(4), "0000".repeat(11));
        parse_hex(b"0041:0000000018242442427E424242420000\n", &mut out);
        parse_hex(wide.as_bytes(), &mut out);
        let a = out[&0x41];
        assert_eq!((a.left, a.right), (1, 6));
        assert_eq!(a.rows[4], 0x18 << 24);
        let yi = out[&0x4E00];
        assert_eq!((yi.left, yi.right), (1, 14));
        // Unparseable lines are skipped.
        parse_hex(b"zz:00\n", &mut out);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn glyph_uvs_have_the_native_texel_inset() {
        for (x, y, w, h) in [(120, 48, 8, 8), (0, 0, 1, 16), (4000, 2000, 32, 16)] {
            let r = glyph_rect(x, y, w, h, (4096, 4096));
            assert_eq!((r.w, r.h), (w, h));
            assert!(r.u0 > x as f32 / 4096.0 && r.v0 > y as f32 / 4096.0);
            assert!(r.u1 < (x + w) as f32 / 4096.0 && r.v1 < (y + h) as f32 / 4096.0);
            assert_eq!(r.u0, (x as f32 + 0.01) / 4096.0);
            assert_eq!(r.v1, ((y + h) as f32 - 0.01) / 4096.0);
        }
    }

    /// A resource root with these font definitions and files.
    fn root(name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let root = std::env::temp_dir().join(format!("horizon-font-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (path, bytes) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        root
    }
    fn white() -> Rect {
        Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 0.0,
            v1: 0.0,
            w: 1,
            h: 1,
        }
    }
    fn load(root: &Path) -> Font {
        Font::load(
            &ResourceSource::directory(root.to_owned()),
            &HashMap::new(),
            &[],
            (4096, 1024),
            (512, 512),
            white(),
        )
    }
    fn png(width: u32, height: u32, opaque: impl Fn(u32, u32) -> bool) -> Vec<u8> {
        let mut pixels = Vec::new();
        for y in 0..height {
            for x in 0..width {
                pixels.extend(if opaque(x, y) { [255; 4] } else { [0; 4] });
            }
        }
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&pixels)
            .unwrap();
        out
    }

    #[test]
    fn archive_fonts_keep_lazy_unihex_on_the_open_reload_snapshot() {
        use std::io::{Cursor, Write};
        let root =
            std::env::temp_dir().join(format!("horizon-font-archive-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let write = |path: &Path, bits: &str| {
            let mut unifont = zip::ZipWriter::new(Cursor::new(Vec::new()));
            unifont
                .start_file("glyphs.hex", zip::write::SimpleFileOptions::default())
                .unwrap();
            unifont
                .write_all(format!("0041:{}\n", bits.repeat(16)).as_bytes())
                .unwrap();
            let unifont = unifont.finish().unwrap().into_inner();
            let image = png(4, 8, |_, _| true);
            let mut pack = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
            for (name, bytes) in [
                ("assets/minecraft/font/default.json", br#"{"providers":[{"type":"reference","id":"demo:nested/bitmap"}]}"#.as_slice()),
                ("assets/demo/font/nested/bitmap.json", br#"{"providers":[{"type":"bitmap","file":"demo:font/sheet.png","ascent":7,"chars":["a"]}]}"#),
                ("assets/demo/font/hex.json", br#"{"providers":[{"type":"unihex","hex_file":"demo:font/unifont.zip"}]}"#),
                ("assets/demo/font/type.json", br#"{"providers":[{"type":"ttf","file":"demo:mono.ttf","size":11}]}"#),
                ("assets/demo/font/mono.ttf", horizon_theme::fonts::GEIST_MONO),
                ("assets/demo/font/unifont.zip", unifont.as_slice()),
                ("assets/demo/textures/font/sheet.png", image.as_slice()),
            ] {
                pack.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
                pack.write_all(bytes).unwrap();
            }
            pack.finish().unwrap();
        };
        let jar = root.join("fonts.jar");
        write(&jar, "FF");
        let source = ResourceSource::archive(&jar).unwrap();
        let font = Font::load(
            &source,
            &HashMap::new(),
            &[],
            (4096, 1024),
            (512, 512),
            white(),
        );
        let metrics = Font::metrics(&source);
        let replacement = root.join("replacement.jar");
        write(&replacement, "F0");
        std::fs::rename(&jar, root.join("old.jar")).unwrap();
        std::fs::rename(replacement, &jar).unwrap();
        assert_eq!(
            font.glyph('a').advance,
            5.0,
            "bitmap font reference in another namespace"
        );
        assert!(
            font.glyph_in("demo:type", 'A').rect.is_some(),
            "archive TrueType data"
        );
        assert_eq!(
            font.glyph_in("demo:hex", 'A').advance,
            5.0,
            "first lazy read uses the old nested archive"
        );
        assert_eq!(metrics.glyph_in("demo:hex", 'A').advance, 5.0);
        assert!(metrics.glyph_in("demo:hex", 'A').rect.is_none());
        let reloaded_source = ResourceSource::archive(&jar).unwrap();
        let reloaded = Font::metrics(&reloaded_source);
        assert_eq!(reloaded.glyph_in("demo:hex", 'A').advance, 3.0);
        assert!(!root.join("assets").exists());
        drop((font, metrics, reloaded, source, reloaded_source));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn server_metrics_preserve_bitmap_true_type_and_unihex_advances() {
        let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
        let def = br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
        let ttf = br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":11,"oversample":2}]}"#;
        let root = root(
            "metrics",
            &[
                ("assets/minecraft/font/default.json", def),
                ("assets/minecraft/font/alt.json", ttf),
                (
                    "assets/minecraft/font/mono.ttf",
                    horizon_theme::fonts::GEIST_MONO,
                ),
                ("assets/minecraft/textures/font/a.png", &sheet),
            ],
        );
        let rendered = load(&root);
        let metrics = Font::metrics(&ResourceSource::directory(root.clone()));
        assert_eq!(metrics.width("a b"), 14.0);
        assert_eq!(metrics.width("§lab"), 12.0);
        let text = Text::literal("ab ab");
        assert_eq!(
            metrics.split_formatted(&text, 10.0),
            rendered.split_formatted(&text, 10.0)
        );
        let alt = Style {
            font: Some("minecraft:alt".into()),
            bold: Some(true),
            ..Style::default()
        };
        for c in ['A', 'i', ' ', '\u{fffd}'] {
            assert_eq!(metrics.advance(c, &alt), rendered.advance(c, &alt));
            assert!(metrics.glyph_styled(c, &alt).rect.is_none());
        }
        let mut hex = HashMap::new();
        parse_hex(b"0041:0000000018242442427E424242420000\n", &mut hex);
        let h = hex[&0x41];
        let glyph = metrics
            .rasterize_hex(&mut metrics.cache.borrow_mut().region, h)
            .unwrap();
        assert_eq!(glyph.advance, 4.0);
        assert_eq!(glyph.offset, 0.5);
        assert!(glyph.rect.is_none());
        assert!(metrics.take_patches().is_empty());
        assert!(!rendered.take_patches().is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn true_type_glyphs_are_rasterized_at_the_provider_size() {
        let def = br#"{"providers":[{"type":"ttf","file":"minecraft:mono.ttf","size":11,"oversample":2,"shift":[0.5,1],"skip":["B"]}]}"#;
        let root = root(
            "ttf",
            &[
                ("assets/minecraft/font/default.json", def),
                (
                    "assets/minecraft/font/mono.ttf",
                    horizon_theme::fonts::GEIST_MONO,
                ),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        let a = font.glyph('A');
        let rect = a.rect.expect("A is drawn");
        // Oversampled 2x: the bitmap is twice the glyph's GUI size.
        assert_eq!(rect.w as f32 / 2.0, a.right - a.left);
        assert_eq!(rect.h as f32 / 2.0, a.bottom - a.top);
        // A monospaced font: every advance is the same, in half pixels.
        assert_eq!(a.advance, font.glyph('i').advance);
        assert_eq!((a.advance * 2.0).fract(), 0.0);
        assert!(a.advance > 4.0 && a.advance < 9.0, "{}", a.advance);
        // Capitals sit on the baseline at y = 7, shifted down a pixel.
        assert!((a.bottom - 8.0).abs() <= 0.5, "{}", a.bottom);
        assert_eq!(a.offset, 1.0);
        // The space has an advance and nothing to draw.
        let space = font.glyph(' ');
        assert!(space.rect.is_none() && space.advance > 0.0);
        // Skipped characters fall through to the missing glyph.
        assert_eq!(font.glyph('B').advance, 6.0);
        // Coverage as `rrrr` texels, faint coverage cut.
        let patches = font.take_patches();
        assert_eq!(patches.len(), 2);
        let texels: Vec<&[u8]> = patches.iter().flat_map(|p| p.rgba.chunks(4)).collect();
        assert!(texels.iter().all(|t| t.iter().all(|v| *v == t[0])));
        assert!(texels.iter().all(|t| t[0] == 0 || t[0] >= 25));
        assert!(texels.iter().any(|t| t[0] == 255));
    }

    /// A bitmap provider's `file` can name any texture. When the atlas has that
    /// texture (HUD packs keep their glyph sheets outside `textures/font`), the
    /// glyphs are read from it instead of being copied into the small run-time
    /// region one by one; the region then cannot fill up and turn later glyphs
    /// into the missing one, whose advance shifts the rest of a HUD line.
    #[test]
    fn a_bitmap_sheet_outside_font_is_read_from_the_atlas() {
        let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
        let (_, _, pixels) = crate::hud::decode_bytes(&sheet).unwrap();
        let rect = Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 16,
            h: 8,
        };
        for (ns, file, key) in [
            ("minecraft", "minecraft:hud/sheet.png", "hud/sheet"),
            (
                "hudengine",
                "hudengine:font/sheet.png",
                "hudengine:font/sheet",
            ),
        ] {
            let def = format!(
                r#"{{"providers":[{{"type":"bitmap","file":"{file}","ascent":7,"chars":["ab"]}}]}}"#
            );
            // The image is only in the atlas: the pack's copy is not on disk.
            let root = root(
                ns,
                &[("assets/minecraft/font/default.json", def.as_bytes())],
            );
            let sprites = HashMap::from([(key.to_string(), rect)]);
            let font = Font::load(
                &ResourceSource::directory(root.clone()),
                &sprites,
                &pixels,
                (16, 8),
                (0, 8),
                white(),
            );
            let _ = std::fs::remove_dir_all(&root);
            assert_eq!(font.glyph('a').advance, 3.0, "{ns}");
            assert_eq!(font.glyph('b').advance, 7.0, "{ns}");
            assert!(font.glyph('a').rect.is_some(), "{ns}");
            assert!(
                font.take_patches().is_empty(),
                "{ns}: the atlas sheet is not copied into the run-time region"
            );
        }
    }

    #[test]
    fn styles_pick_their_font_and_unknown_fonts_draw_missing_glyphs() {
        let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
        let default = br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
        let alt = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ba"]}]}"#;
        let root = root(
            "styles",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/minecraft/font/alt.json", alt),
                ("assets/minecraft/textures/font/a.png", &sheet),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        // The sheet is not in this atlas: glyphs are copied in when first drawn.
        assert_eq!(font.glyph('a').advance, 3.0);
        assert_eq!(font.glyph('b').advance, 7.0);
        let alt = Style {
            font: Some("minecraft:alt".into()),
            ..Style::default()
        };
        assert_eq!(font.glyph_styled('a', &alt).advance, 7.0);
        assert_eq!(font.glyph_in("alt", 'b').advance, 3.0);
        assert!(font.glyph_styled('a', &alt).rect.is_some());
        // Not in alt (no space provider there), or no such font at all.
        assert_eq!(font.glyph_styled(' ', &alt).advance, 6.0);
        let missing = Style {
            font: Some("minecraft:nope".into()),
            ..Style::default()
        };
        assert_eq!(font.glyph_styled(' ', &missing).advance, 6.0);
        assert_eq!(font.advance('a', &alt), 7.0);
        let text = Text::literal("ab").with_style(&alt);
        assert_eq!(font.width_text(&text), 10.0);
        assert_eq!(font.random_glyph(&alt, 7.0).advance, 7.0);
        assert!(!font.take_patches().is_empty());
    }

    /// `FontSet.computeGlyphInfo`: `SelectedGlyphs.any` is the first provider's glyph, fishy
    /// advance or not, and is what is drawn and measured; `nonFishy`, what obfuscated text draws,
    /// is the first provider whose advance is within 0..32, else the missing glyph.
    #[test]
    fn a_fishy_glyph_is_drawn_and_only_obfuscated_text_falls_through() {
        // 64 opaque columns: the first provider's 'a' advances 65, past the 32 vanilla
        // calls fishy, and the narrow provider's 'a' advances 3.
        let wide = png(64, 8, |_, _| true);
        let narrow = png(16, 8, |x, _| x < 2);
        let def = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/wide.png","ascent":7,"chars":["a"]},{"type":"bitmap","file":"minecraft:font/narrow.png","ascent":7,"chars":["a"]}]}"#;
        let root = root(
            "fishy",
            &[
                ("assets/minecraft/font/default.json", def),
                ("assets/minecraft/textures/font/wide.png", &wide),
                ("assets/minecraft/textures/font/narrow.png", &narrow),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        let drawn = font.glyph('a');
        assert_eq!(drawn.advance, 65.0, "the first provider's glyph is drawn");
        assert!(drawn.rect.is_some());
        assert_eq!(font.advance('a', &Style::default()), 65.0);
        // The by-width table files 'a' under its first glyph's 65; the glyph it then draws is
        // the narrow provider's (`nonFishy`).
        assert_eq!(font.random_glyph(&Style::default(), 65.0).advance, 3.0);
        // Nothing else is 3 wide in the table, and the missing glyph is 6.
        assert_eq!(font.random_glyph(&Style::default(), 3.0).advance, 6.0);
    }

    /// A `space` provider's shift glyphs (kerning in server packs: -512 to 512) are drawn and
    /// measured as they are, and `nonFishy` has nothing else to draw for them.
    #[test]
    fn space_glyphs_with_any_advance_are_kept() {
        let shifts: Vec<i32> = (0..10)
            .map(|i| -(1 << i))
            .chain((0..10).map(|i| 1 << i))
            .collect();
        let advances: Vec<String> = shifts
            .iter()
            .enumerate()
            .map(|(i, a)| format!("\"\\ue{:03x}\": {a}", i))
            .collect();
        let def = format!(
            r#"{{"providers":[{{"type":"space","advances":{{{}}}}}]}}"#,
            advances.join(",")
        );
        let root = root(
            "space-shifts",
            &[("assets/minecraft/font/default.json", def.as_bytes())],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        for (i, a) in shifts.iter().enumerate() {
            let c = char::from_u32(0xe000 + i as u32).unwrap();
            assert_eq!(font.glyph(c).advance, *a as f32, "{c:?}");
            assert_eq!(font.advance(c, &Style::default()), *a as f32);
        }
        // Obfuscated text asks for a glyph of a given width: a fishy space has none to give.
        let c = char::from_u32(0xe000 + 19).unwrap();
        assert_eq!(font.glyph(c).advance, 512.0);
        assert_eq!(
            font.glyph_non_fishy(DEFAULT, c).advance,
            6.0,
            "the missing glyph"
        );
        assert_eq!(font.random_glyph(&Style::default(), 512.0).advance, 6.0);
    }

    /// Nexo's GUI glyphs: a bitmap `height` of 287 with no advance, so every glyph wider than 31
    /// pixels has a fishy advance, and is still drawn.
    #[test]
    fn a_wide_bitmap_glyph_is_drawn() {
        let sheet = png(40, 287, |_, _| true);
        let def = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/gui.png","height":287,"ascent":65,"chars":["a"]}]}"#;
        let root = root(
            "wide-bitmap",
            &[
                ("assets/minecraft/font/default.json", def),
                ("assets/minecraft/textures/font/gui.png", &sheet),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        let g = font.glyph('a');
        assert_eq!(g.advance, 41.0, "the glyph's 40 columns and one");
        assert!(g.rect.is_some(), "drawn, not the missing glyph");
        assert!(fishy_advance(g.advance));
        assert_eq!(
            font.glyph_non_fishy(DEFAULT, 'a').advance,
            6.0,
            "excluded from what obfuscated text draws"
        );
    }

    /// A font whose first provider is `first` (its sheet `sheet`), then a
    /// valid provider drawing `a` from a sheet with two columns used.
    fn font_after(name: &str, first: &str, sheet: &[u8]) -> Font {
        let narrow = png(8, 8, |x, _| x < 2);
        let def = format!(
            r#"{{"providers":[{first},{{"type":"bitmap","file":"minecraft:font/narrow.png","ascent":7,"chars":["a"]}}]}}"#
        );
        let root = root(
            name,
            &[
                ("assets/minecraft/font/default.json", def.as_bytes()),
                ("assets/minecraft/textures/font/first.png", sheet),
                ("assets/minecraft/textures/font/narrow.png", &narrow),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        font
    }

    /// `BitmapProvider.Definition` refuses these while the file decodes, and
    /// the whole file goes. Rows of unequal length were sized from the first
    /// row, so the longer row's last cell reached past the sheet and panicked
    /// when drawn.
    #[test]
    fn a_font_file_with_a_refused_bitmap_provider_is_skipped() {
        let sheet = png(16, 16, |_, _| true);
        for (name, first) in [
            ("uneven", r#""ascent":7,"chars":["ab","cdef"]"#),
            (
                "ascent-above-height",
                r#""height":8,"ascent":9,"chars":["af"]"#,
            ),
            ("text-ascent", r#""ascent":"7","chars":["af"]"#),
            ("no-ascent", r#""chars":["af"]"#),
            ("empty-row", r#""ascent":7,"chars":["","af"]"#),
        ] {
            let first = format!(r#"{{"type":"bitmap","file":"minecraft:font/first.png",{first}}}"#);
            let font = font_after(name, &first, &sheet);
            let missing = font.glyph('z');
            for c in ['a', 'f'] {
                let g = font.glyph(c);
                assert_eq!(
                    g.advance, missing.advance,
                    "{name}: {c} is the missing glyph"
                );
                assert_eq!(g.rect.map(|r| r.u0), missing.rect.map(|r| r.u0), "{name}");
            }
        }
    }

    /// Vanilla reads `height` and `ascent` as integers, a fraction truncated
    /// toward zero: 8.9 and 6.5 load as 8 and 6.
    #[test]
    fn fractional_height_and_ascent_are_truncated() {
        let sheet = png(16, 16, |_, _| true);
        let first = r#"{"type":"bitmap","file":"minecraft:font/first.png","height":8.9,"ascent":6.5,"chars":["af"]}"#;
        let g = font_after("fractional", first, &sheet).glyph('f');
        // Cells 8x16 at height 8: half scale; top 7 - 6.
        assert_eq!((g.top, g.bottom), (1.0, 9.0));
        assert_eq!(g.advance, 5.0, "8 columns used, at half scale, and one");
    }

    /// `BitmapProvider.Definition.load` puts each cell in a map: a character
    /// listed twice keeps its last cell. Character 0 is padding, and an earlier
    /// provider still wins over a later one.
    #[test]
    fn a_repeated_character_keeps_its_last_cell() {
        // Cells 8 wide using 2, 6 and 4 columns: advances 3, 7 and 5.
        let sheet = png(24, 8, |x, _| {
            x < 2 || (8..14).contains(&x) || (16..20).contains(&x)
        });
        let first = r#"{"type":"bitmap","file":"minecraft:font/first.png","ascent":7,"chars":["a\u0000a"]}"#;
        let font = font_after("repeated", first, &sheet);
        assert_eq!(
            font.glyph('a').advance,
            5.0,
            "the last cell, not the first (3) or the next provider's (3)"
        );
        assert_eq!(
            font.glyph('\0').advance,
            font.glyph('z').advance,
            "character 0 is not the middle cell (7)"
        );
    }

    /// More rows or columns than the sheet has pixels leave cells of no size.
    /// Vanilla keeps the provider: each character is an empty glyph of advance
    /// 1, and a later provider's glyph for it isn't used.
    #[test]
    fn a_grid_larger_than_its_sheet_draws_empty_glyphs() {
        for (name, sheet, chars) in [
            ("too-many-rows", png(16, 1, |_, _| true), r#"["a","b"]"#),
            ("too-many-columns", png(4, 8, |_, _| true), r#"["abcde"]"#),
        ] {
            let first = format!(
                r#"{{"type":"bitmap","file":"minecraft:font/first.png","ascent":7,"chars":{chars}}}"#
            );
            let g = font_after(name, &first, &sheet).glyph('a');
            assert_eq!(g.advance, 1.0, "{name}: not the next provider's 3");
            assert!(g.rect.is_none(), "{name}: nothing drawn");
            assert!(g.top.is_finite() && g.bottom.is_finite(), "{name}");
        }
    }

    /// StringDecomposer.iterateFormatted: a `§` that ends a part stops that
    /// part only; the next part is still measured and can break.
    #[test]
    fn a_section_sign_ending_a_part_still_breaks_the_next() {
        let sheet = png(16, 8, |x, _| x < 2);
        let default = br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
        let root = root(
            "section-sign",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/minecraft/textures/font/a.png", &sheet),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        let text = Text::from_spans(
            ["a§", "a a"]
                .map(|part| Span::new(part, Style::default()))
                .into(),
        );
        // 3 + 3 + 4 + 3 is past 12: the line breaks at the space.
        let lines: Vec<String> = font
            .split_formatted(&text, 12.0)
            .iter()
            .map(Text::plain)
            .collect();
        assert_eq!(lines, ["a§a", "a"]);
        let mixed = Text::literal("aa")
            .append(Text::untrusted("aa"))
            .with_style(&Style {
                click: Some(crate::text::ClickEvent::OpenFile("/tmp/local".into())),
                ..Style::default()
            });
        let lines = font.split_formatted(&mixed, 6.0);
        assert_eq!(
            lines.iter().map(Text::plain).collect::<Vec<_>>(),
            ["aa", "aa"]
        );
        assert_eq!(lines[0].spans[0].origin(), crate::text::Origin::Local);
        assert_eq!(lines[1].spans[0].origin(), crate::text::Origin::Remote);
        assert!(lines[0].spans[0].click_event().is_some());
        assert!(lines[1].spans[0].click_event().is_none());
    }

    /// A renderer rebuilt over the same font (the loading screen's handoff)
    /// makes its atlas texture from the textures alone, so the glyphs it had
    /// rasterized into the previous texture must be placed into the new one.
    #[test]
    fn a_rebuilt_atlas_gets_its_glyphs_again() {
        let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
        let default = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
        let root = root(
            "rebuild",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/minecraft/textures/font/a.png", &sheet),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        let first = font.glyph('a');
        assert_eq!(font.take_patches().len(), 1);
        font.reset_cache();
        // Nothing is left over: drawing the character again places its pixels.
        assert!(font.take_patches().is_empty());
        assert_eq!(font.glyph('a').advance, first.advance);
        assert_eq!(font.take_patches().len(), 1);
    }

    #[test]
    fn provider_filters_follow_the_font_options() {
        let sheet = png(8, 8, |x, _| x < 4);
        let default = br#"{"providers":[{"type":"reference","id":"minecraft:include/sheet","filter":{"uniform":false}},{"type":"space","advances":{"a":2}}]}"#;
        let sheet_def = br#"{"providers":[{"type":"bitmap","file":"minecraft:font/s.png","ascent":7,"chars":["a"]}]}"#;
        let root = root(
            "filters",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/minecraft/font/include/sheet.json", sheet_def),
                ("assets/minecraft/textures/font/s.png", &sheet),
            ],
        );
        let mut font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(font.glyph('a').advance, 5.0);
        font.uniform = true;
        font.reset_cache();
        assert_eq!(font.glyph('a').advance, 2.0);
    }

    #[test]
    fn a_full_glyph_region_recovers_at_the_next_frame() {
        // Two 8x8 glyphs from a sheet outside the atlas; the region holds one row.
        let sheet = png(16, 8, |x, _| x % 8 < 4);
        let default = br#"{"providers":[{"type":"bitmap","file":"pack:font/s.png","ascent":7,"chars":["ab"]}]}"#;
        let root = root(
            "full",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/pack/textures/font/s.png", &sheet),
            ],
        );
        // A 10 wide atlas fits one 8 pixel glyph per row; the region two rows.
        let font = Font::load(
            &ResourceSource::directory(root.clone()),
            &HashMap::new(),
            &[],
            (10, 64),
            (0, 8),
            white(),
        );
        let _ = std::fs::remove_dir_all(&root);
        let epoch = font.epoch();
        let a = font.glyph('a');
        assert!(a.rect.is_some());
        // No room for 'b': missing for now, and asked again next time.
        assert!(font.glyph('b').rect.is_none());
        assert!(font.glyph('b').rect.is_none());
        assert_eq!(font.epoch(), epoch);
        font.begin_frame();
        assert_ne!(font.epoch(), epoch);
        let b = font.glyph('b');
        assert!(b.rect.is_some());
        assert_eq!(b.advance, a.advance);
        // Nothing overflowed since: the next frame keeps the glyphs.
        let epoch = font.epoch();
        font.begin_frame();
        assert_eq!(font.epoch(), epoch);
    }

    #[test]
    fn a_character_without_a_provider_does_not_reset_the_region() {
        let sheet = png(8, 8, |x, _| x < 4);
        let default = br#"{"providers":[{"type":"bitmap","file":"pack:font/s.png","ascent":7,"chars":["a"]}]}"#;
        let root = root(
            "nofill",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/pack/textures/font/s.png", &sheet),
            ],
        );
        let font = Font::load(
            &ResourceSource::directory(root.clone()),
            &HashMap::new(),
            &[],
            (10, 64),
            (0, 8),
            white(),
        );
        let _ = std::fs::remove_dir_all(&root);
        let epoch = font.epoch();
        assert!(font.glyph('z').rect.is_none());
        font.begin_frame();
        assert_eq!(font.epoch(), epoch);
    }

    #[test]
    fn ascii_cache_text_width_and_line_splitting_match_legacy_and_multi_span_rules() {
        let sheet = png(16, 8, |x, _| x < 2 || (8..14).contains(&x));
        let default = br#"{"providers":[{"type":"space","advances":{" ":4}},{"type":"bitmap","file":"minecraft:font/a.png","ascent":7,"chars":["ab"]}]}"#;
        let root = root(
            "ascii-split",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/minecraft/textures/font/a.png", &sheet),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        // 'a' advance = 3, 'b' advance = 7, ' ' advance = 4.
        assert_eq!(font.width("a b"), 14.0);
        assert_eq!(font.width("§lab"), 12.0);
        assert_eq!(font.width("§la§rb"), 11.0);
        assert_eq!(font.width("a§"), 3.0);
        assert_eq!(font.fit("aba", 10.0), 2);
        assert_eq!(font.fit_reverse("aba", 10.0), 2);
        assert_eq!(font.fit("aba", 100.0), 3);
        assert_eq!(font.fit_reverse("aba", 100.0), 3);
        // split_lines over multiple spans with trailing newline and wrap flags.
        let text = Text::literal("aa ")
            .append(Text::literal("bb\n"))
            .append(Text::literal("a\n"));
        let lines = font.split_lines(&text, 10.0);
        let summary: Vec<(String, bool)> = lines
            .iter()
            .map(|(line, wrapped)| (line.plain(), *wrapped))
            .collect();
        assert_eq!(
            summary,
            [
                ("aa".into(), false),
                ("b".into(), true),
                ("b".into(), true),
                ("a".into(), false),
                ("".into(), false),
            ]
        );
        // split_formatted carries legacy bold across line break inside the same part.
        let formatted = Text::literal("§laa aa");
        let split = font.split_formatted(&formatted, 8.0);
        assert_eq!(split.len(), 2);
        assert_eq!(split[0].plain(), "§laa");
        assert_eq!(split[1].plain(), "aa");
        assert!(split[1].spans[0].style.is_bold());
    }

    #[test]
    fn off_atlas_large_bitmap_sheet_compresses_strips_and_round_trips_pixels() {
        // 128x16 sheet (1 row of 16x16 cells = 8192 raw RGBA bytes -> LZ4-compressed strip).
        let sheet = png(128, 16, |x, y| x < 12 && y < 12);
        let default = br#"{"providers":[{"type":"bitmap","file":"pack:font/big.png","height":8,"ascent":7,"chars":["abcdefgh"]}]}"#;
        let root = root(
            "lz4-strip",
            &[
                ("assets/minecraft/font/default.json", default),
                ("assets/pack/textures/font/big.png", &sheet),
            ],
        );
        let font = load(&root);
        let _ = std::fs::remove_dir_all(&root);
        let a = font.glyph('a');
        assert!(a.rect.is_some());
        assert_eq!(a.advance, 7.0);
        let patches = font.take_patches();
        assert_eq!(patches.len(), 1);
        assert_eq!((patches[0].width, patches[0].height), (16, 16));
        // Top-left pixel opaque white, bottom-right transparent.
        assert_eq!(&patches[0].rgba[0..4], &[255, 255, 255, 255]);
        let last = patches[0].rgba.len() - 4;
        assert_eq!(&patches[0].rgba[last..], &[0, 0, 0, 0]);
    }
}
