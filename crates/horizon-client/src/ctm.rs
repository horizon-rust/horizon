//! Connected textures in OptiFine's format (`assets/<namespace>/optifine/ctm/**/*.properties`,
//! as OptiFine and Continuity read them): a rule swaps the sprite of some blocks' faces for one
//! of its tiles, picked from which neighbours in the face's plane connect. Horizon draws
//! `method=ctm` (the 47-tile template, `glass` its old name) with `connect` (`block`, `tile`,
//! `state`), `faces`, `innerSeams`, `matchBlocks` (with properties) and `matchTiles`, the
//! methods SPBR uses; a rule of another method is skipped with a warning.
//!
//! The mesher asks [`Ctm::tile`] for every face it draws. Rules are tried in file order, rules
//! matching a sprite before rules matching a block, the first that applies wins. A rule with
//! `matchBlocks` and no `matchTiles` changes the faces drawn with the block's particle sprite
//! (a glass pane's panes but not its edges).

use crate::{Result, sprites::Sprites};
use horizon_assets::model::Direction;
use horizon_world::registry::{BlockRegistry, BlockStateId};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Which neighbours of `method=ctm`'s 47 tiles connect: edges `U`, `R`, `D`, `L` (up, right,
/// down and left in the texture), then the corners that connect too, `a` to `d` (up-left,
/// up-right, down-right, down-left; a corner counts only between two connected edges). Tile
/// `n` is `TILES[n]`, the layout of OptiFine's template.
const TILES: [&str; 47] = [
    "",
    "R",
    "RL",
    "L",
    "RD",
    "DL",
    "URD",
    "RDL",
    "URDL b",
    "URDL c",
    "URDL ad",
    "URDL ab",
    "D",
    "RD c",
    "RDL cd",
    "DL d",
    "UR",
    "UL",
    "URL",
    "UDL",
    "URDL a",
    "URDL d",
    "URDL cd",
    "URDL bc",
    "UD",
    "URD bc",
    "URDL abcd",
    "UDL ad",
    "URD c",
    "RDL d",
    "URD b",
    "RDL c",
    "URDL abd",
    "URDL abc",
    "URDL bd",
    "URDL ac",
    "U",
    "UR b",
    "URL ab",
    "UL a",
    "URL b",
    "UDL a",
    "URL a",
    "UDL d",
    "URDL acd",
    "URDL bcd",
    "URDL",
];

const UP: u8 = 1;
const RIGHT: u8 = 2;
const DOWN: u8 = 4;
const LEFT: u8 = 8;
/// Corner bits, each between two edges: up-left, up-right, down-right, down-left.
const CORNERS: [(u8, u8, u8); 4] = [
    (16, UP, LEFT),
    (32, UP, RIGHT),
    (64, DOWN, RIGHT),
    (128, DOWN, LEFT),
];

/// A [`TILES`] entry's connection bits.
fn tile_key(tile: &str) -> u8 {
    tile.bytes().fold(0, |key, c| {
        key | match c {
            b'U' => UP,
            b'R' => RIGHT,
            b'D' => DOWN,
            b'L' => LEFT,
            b'a' => CORNERS[0].0,
            b'b' => CORNERS[1].0,
            b'c' => CORNERS[2].0,
            b'd' => CORNERS[3].0,
            _ => 0,
        }
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Connect {
    /// The neighbour is the same block.
    Block,
    /// The neighbour is drawn by the same rule.
    Tile,
    /// The neighbour is the same block state.
    State,
}

/// A tile: a sprite layer, or the face's own sprite (`<default>`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tile {
    Layer(u32),
    Default,
    /// `<skip>`: this rule does not draw the face, the next may.
    Skip,
}

#[derive(Debug)]
struct Rule {
    tiles: Vec<Tile>,
    /// Bit `Direction as usize` for the faces it changes.
    faces: u8,
    connect: Connect,
    inner_seams: bool,
    /// `matchTiles`' sprite layers (empty: the block's particle sprite).
    sprites: Vec<u32>,
}

/// The connected-texture rules of the resource packs, ready for the mesher.
#[derive(Debug, Default)]
pub struct Ctm {
    rules: Vec<Rule>,
    /// By state id: the rules naming its block, in order, with the sprite each changes.
    by_state: Vec<Vec<(u16, u32)>>,
    /// Rules that name only sprites, by sprite layer.
    by_sprite: HashMap<u32, Vec<u16>>,
    /// By state id: its block's index, for `connect=block`.
    block_of: Vec<u32>,
    /// Connection bits to tile index.
    tile_of: Vec<u8>,
}

impl Ctm {
    /// Whether no rule loaded (the mesher then skips every lookup).
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
    /// How many rules loaded.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// The rules under `root`'s `optifine/ctm`, their tiles added to `sprites`;
    /// `particle(state)` is a state's particle sprite layer. A rule that does not load is
    /// skipped with a warning, as OptiFine does.
    pub fn load(
        root: &Path,
        registry: &BlockRegistry,
        sprites: &mut Sprites,
        particle: impl Fn(BlockStateId) -> Option<u32>,
    ) -> Self {
        let files = rule_files(root);
        if files.is_empty() {
            return Self::default();
        }
        let mut names: HashMap<&str, u32> = HashMap::new();
        let block_of: Vec<u32> = (0..registry.len() as u32)
            .map(|id| {
                let name = registry
                    .state(BlockStateId(id))
                    .map_or("", |s| s.name.as_str());
                let next = names.len() as u32;
                *names.entry(name).or_insert(next)
            })
            .collect();
        let mut tile_of = vec![0u8; 256];
        for (index, tile) in TILES.iter().enumerate() {
            tile_of[tile_key(tile) as usize] = index as u8;
        }
        let mut ctm = Self {
            by_state: vec![Vec::new(); registry.len()],
            block_of,
            tile_of,
            ..Default::default()
        };
        // OptiFine tries rules matching sprites before rules matching blocks.
        let mut parsed: Vec<ParsedRule> = Vec::new();
        for (namespace, path) in files {
            match ParsedRule::load(root, &namespace, &path, sprites) {
                Ok(Some(rule)) => parsed.push(rule),
                Ok(None) => {}
                Err(e) => log::warn!(
                    "Horizon: skipping the connected textures {}: {e}",
                    path.display()
                ),
            }
        }
        parsed.sort_by_key(|rule| !rule.match_blocks.is_empty());
        for rule in parsed {
            let index = ctm.rules.len() as u16;
            let mut matched = 0;
            for id in 0..registry.len() as u32 {
                let Some(state) = registry.state(BlockStateId(id)) else {
                    continue;
                };
                if !rule
                    .match_blocks
                    .iter()
                    .any(|m| m.matches(&state.name, &state.properties))
                {
                    continue;
                }
                let sprite = if rule.sprites.is_empty() {
                    match particle(BlockStateId(id)) {
                        Some(layer) => layer,
                        None => continue,
                    }
                } else {
                    u32::MAX
                };
                ctm.by_state[id as usize].push((index, sprite));
                matched += 1;
            }
            if rule.match_blocks.is_empty() {
                for &layer in &rule.sprites {
                    ctm.by_sprite.entry(layer).or_default().push(index);
                }
            } else if matched == 0 {
                log::debug!("Horizon: connected textures {} match no block", rule.file);
            }
            ctm.rules.push(Rule {
                tiles: rule.tiles,
                faces: rule.faces,
                connect: rule.connect,
                inner_seams: rule.inner_seams,
                sprites: rule.sprites,
            });
        }
        ctm
    }

    /// The sprite layer to draw a face of `state` with instead of its own `layer`, or `None`
    /// to keep it. `facing` is the face's direction, `positions` and `uv` its quad (they give
    /// which way the texture's up and right point), and `state_at(dx, dy, dz)` the state of
    /// a block next to it (each offset -1 to 1).
    pub fn tile(
        &self,
        state: u32,
        layer: u32,
        facing: Direction,
        positions: &[[f32; 3]; 4],
        uv: &[[f32; 2]; 4],
        state_at: impl Fn([i32; 3]) -> u32,
    ) -> Option<u32> {
        let by_state = self
            .by_state
            .get(state as usize)
            .map_or(&[][..], Vec::as_slice);
        let by_sprite = match self.by_sprite.is_empty() {
            true => &[][..],
            false => self.by_sprite.get(&layer).map_or(&[][..], Vec::as_slice),
        };
        if by_state.is_empty() && by_sprite.is_empty() {
            return None;
        }
        // Rules naming sprites only, then those naming the block (with its particle sprite).
        let candidates = by_sprite
            .iter()
            .map(|&r| (r, None))
            .chain(by_state.iter().map(|&(r, particle)| (r, Some(particle))));
        for (index, particle) in candidates {
            let rule = &self.rules[index as usize];
            let applies = match particle {
                Some(particle) if rule.sprites.is_empty() => particle == layer,
                _ => rule.sprites.contains(&layer),
            };
            if !applies || rule.faces & (1 << facing as u8) == 0 {
                continue;
            }
            let key = self.connections(index, rule, state, facing, positions, uv, &state_at);
            match rule.tiles[self.tile_of[key as usize] as usize] {
                Tile::Layer(tile) => return Some(tile),
                Tile::Default => return None,
                Tile::Skip => continue,
            }
        }
        None
    }

    /// Whether the block of `other` connects to `state` under `rule`.
    fn connects(&self, index: u16, rule: &Rule, state: u32, other: u32) -> bool {
        match rule.connect {
            Connect::State => other == state,
            Connect::Block => {
                self.block_of.get(other as usize) == self.block_of.get(state as usize)
            }
            Connect::Tile => self
                .by_state
                .get(other as usize)
                .is_some_and(|rules| rules.iter().any(|&(r, _)| r == index)),
        }
    }

    /// The face's connection bits ([`TILES`]' edges and corners).
    #[allow(clippy::too_many_arguments)]
    fn connections(
        &self,
        index: u16,
        rule: &Rule,
        state: u32,
        facing: Direction,
        positions: &[[f32; 3]; 4],
        uv: &[[f32; 2]; 4],
        state_at: &impl Fn([i32; 3]) -> u32,
    ) -> u8 {
        let (right, down) = texture_axes(facing, positions, uv);
        let front = normal(facing);
        let add = |a: [i32; 3], b: [i32; 3]| [a[0] + b[0], a[1] + b[1], a[2] + b[2]];
        let neg = |a: [i32; 3]| a.map(|v| -v);
        let connected = |offset: [i32; 3]| {
            self.connects(index, rule, state, state_at(offset))
                // innerSeams: a neighbour whose face is covered by the same kind of block
                // does not connect.
                && !(rule.inner_seams
                    && self.connects(index, rule, state, state_at(add(offset, front))))
        };
        let edges = [
            (UP, neg(down)),
            (RIGHT, right),
            (DOWN, down),
            (LEFT, neg(right)),
        ];
        let mut key = 0;
        for (bit, offset) in edges {
            if connected(offset) {
                key |= bit;
            }
        }
        let offset_of = |bit: u8| edges.iter().find(|e| e.0 == bit).map(|e| e.1).unwrap();
        for (bit, a, b) in CORNERS {
            if key & a != 0 && key & b != 0 && connected(add(offset_of(a), offset_of(b))) {
                key |= bit;
            }
        }
        key
    }
}

/// The block-space offsets of one step right and one step down the texture on a face, from
/// its quad's texture coordinates; a quad whose coordinates give none uses vanilla's mapping
/// of a cube's faces.
fn texture_axes(facing: Direction, p: &[[f32; 3]; 4], uv: &[[f32; 2]; 4]) -> ([i32; 3], [i32; 3]) {
    let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    let (e1, e2) = (sub(p[1], p[0]), sub(p[2], p[0]));
    let (t1, t2) = (
        [uv[1][0] - uv[0][0], uv[1][1] - uv[0][1]],
        [uv[2][0] - uv[0][0], uv[2][1] - uv[0][1]],
    );
    let r = t1[0] * t2[1] - t2[0] * t1[1];
    let normal = normal(facing);
    let axis = |v: [f32; 3]| -> Option<[i32; 3]> {
        let (i, value) = v
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))?;
        if value.abs() < 1e-6 || normal[i] != 0 {
            return None;
        }
        let mut out = [0; 3];
        out[i] = if *value > 0.0 { 1 } else { -1 };
        Some(out)
    };
    if r.abs() > 1e-9 {
        let tangent: [f32; 3] = std::array::from_fn(|i| (e1[i] * t2[1] - e2[i] * t1[1]) / r);
        let bitangent: [f32; 3] = std::array::from_fn(|i| (e2[i] * t1[0] - e1[i] * t2[0]) / r);
        if let (Some(right), Some(down)) = (axis(tangent), axis(bitangent))
            && right != down
        {
            return (right, down);
        }
    }
    match facing {
        Direction::Up => ([1, 0, 0], [0, 0, 1]),
        Direction::Down => ([1, 0, 0], [0, 0, -1]),
        Direction::North => ([-1, 0, 0], [0, -1, 0]),
        Direction::South => ([1, 0, 0], [0, -1, 0]),
        Direction::West => ([0, 0, 1], [0, -1, 0]),
        Direction::East => ([0, 0, -1], [0, -1, 0]),
    }
}

/// The block offset `d` faces.
fn normal(d: Direction) -> [i32; 3] {
    match d {
        Direction::Down => [0, -1, 0],
        Direction::Up => [0, 1, 0],
        Direction::North => [0, 0, -1],
        Direction::South => [0, 0, 1],
        Direction::West => [-1, 0, 0],
        Direction::East => [1, 0, 0],
    }
}

/// Every `.properties` file under each namespace's `optifine/ctm`, with its namespace, in
/// path order.
fn rule_files(root: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(namespaces) = std::fs::read_dir(root.join("assets")) else {
        return out;
    };
    for namespace in namespaces.flatten() {
        let name = namespace.file_name().to_string_lossy().into_owned();
        let mut stack = vec![namespace.path().join("optifine/ctm")];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "properties") {
                    out.push((name.clone(), path));
                }
            }
        }
    }
    out.sort();
    out
}

/// `key=value` lines of a Java properties file (`=` or `:` separates, `#` and `!` start
/// comments, a trailing `\` continues the line).
pub(crate) fn properties(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut pending = String::new();
    for line in text.lines() {
        let line = line.trim_start();
        if pending.is_empty() && (line.starts_with('#') || line.starts_with('!')) {
            continue;
        }
        if let Some(rest) = line.strip_suffix('\\') {
            pending.push_str(rest);
            continue;
        }
        pending.push_str(line);
        let entry = std::mem::take(&mut pending);
        let Some(split) = entry.find(['=', ':']) else {
            continue;
        };
        let (key, value) = entry.split_at(split);
        out.insert(key.trim().to_owned(), value[1..].trim().to_owned());
    }
    out
}

/// `matchBlocks` entry: `[namespace:]name[:property=value,value...]...`.
#[derive(Debug, PartialEq)]
struct BlockMatch {
    name: String,
    properties: Vec<(String, Vec<String>)>,
}
impl BlockMatch {
    fn parse(entry: &str) -> Option<Self> {
        let mut parts = entry.split(':').peekable();
        let first = parts.next()?;
        let name = match parts.peek() {
            Some(second) if !second.contains('=') => {
                let name = format!("{first}:{second}");
                parts.next();
                name
            }
            _ => format!("minecraft:{first}"),
        };
        let properties = parts
            .map(|p| {
                let (key, values) = p.split_once('=')?;
                Some((
                    key.to_owned(),
                    values.split(',').map(str::to_owned).collect(),
                ))
            })
            .collect::<Option<_>>()?;
        Some(Self { name, properties })
    }
    fn matches(&self, name: &str, properties: &BTreeMap<String, String>) -> bool {
        self.name == name
            && self.properties.iter().all(|(key, values)| {
                properties
                    .get(key)
                    .is_some_and(|value| values.iter().any(|v| v == value))
            })
    }
}

struct ParsedRule {
    file: String,
    tiles: Vec<Tile>,
    faces: u8,
    connect: Connect,
    inner_seams: bool,
    sprites: Vec<u32>,
    match_blocks: Vec<BlockMatch>,
}

impl ParsedRule {
    /// The rule in `path`, a file of `namespace`; `None` for a method Horizon does not draw.
    fn load(
        root: &Path,
        namespace: &str,
        path: &Path,
        sprites: &mut Sprites,
    ) -> Result<Option<Self>> {
        let file = path.display().to_string();
        let keys = properties(&std::fs::read_to_string(path)?);
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let method = keys.get("method").map_or("", String::as_str);
        if !matches!(method, "ctm" | "glass") {
            log::warn!(
                "Horizon: connected textures {file}: method {method:?} is not drawn yet; skipped"
            );
            return Ok(None);
        }
        // Without either, OptiFine reads them from the file name.
        let mut match_blocks: Vec<BlockMatch> = keys
            .get("matchBlocks")
            .map(|v| v.split_whitespace().filter_map(BlockMatch::parse).collect())
            .unwrap_or_default();
        let mut match_tiles: Vec<String> = keys
            .get("matchTiles")
            .map(|v| v.split_whitespace().map(str::to_owned).collect())
            .unwrap_or_default();
        if match_blocks.is_empty() && match_tiles.is_empty() {
            match stem.strip_prefix("block_") {
                Some(block) => match_blocks.extend(BlockMatch::parse(block)),
                None => match_tiles.push(stem.to_owned()),
            }
        }
        let tiles = tile_names(keys.get("tiles").map_or("", String::as_str))
            .into_iter()
            .map(|name| match name.as_str() {
                "<skip>" => Ok(Tile::Skip),
                "<default>" => Ok(Tile::Default),
                _ => {
                    let (key, file) = tile_file(root, namespace, path, &name);
                    sprites.insert_file(&key, &file).map(Tile::Layer)
                }
            })
            .collect::<Result<Vec<_>>>()?;
        if tiles.len() < TILES.len() {
            return Err(format!("method=ctm needs 47 tiles, {} given", tiles.len()).into());
        }
        let sprites = match_tiles
            .iter()
            .map(|tile| {
                let name = if tile.contains(':') {
                    tile.clone()
                } else if tile.contains('/') {
                    format!("minecraft:{tile}")
                } else {
                    format!("minecraft:block/{tile}")
                };
                sprites.insert(root, name.trim_end_matches(".png"))
            })
            .collect::<Result<Vec<_>>>()?;
        let faces = match keys.get("faces") {
            None => 0b11_1111,
            Some(v) => v.split_whitespace().fold(0, |mask, face| {
                mask | match face {
                    "bottom" => 1 << Direction::Down as u8,
                    "top" => 1 << Direction::Up as u8,
                    "north" => 1 << Direction::North as u8,
                    "south" => 1 << Direction::South as u8,
                    "west" => 1 << Direction::West as u8,
                    "east" => 1 << Direction::East as u8,
                    "sides" => 0b11_1100,
                    "all" => 0b11_1111,
                    _ => 0,
                }
            }),
        };
        let connect = match keys.get("connect").map(String::as_str) {
            Some("tile") => Connect::Tile,
            Some("state") => Connect::State,
            Some("block") => Connect::Block,
            _ if match_blocks.is_empty() => Connect::Tile,
            _ => Connect::Block,
        };
        Ok(Some(Self {
            file,
            tiles,
            faces,
            connect,
            inner_seams: keys.get("innerSeams").is_some_and(|v| v == "true"),
            sprites,
            match_blocks,
        }))
    }
}

/// `tiles`' entries in order: `0-3` expands to `0 1 2 3`.
fn tile_names(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    for token in value.split_whitespace() {
        if let Some((a, b)) = token.split_once('-')
            && let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>())
        {
            out.extend((a..=b).map(|n| n.to_string()));
            continue;
        }
        out.push(token.to_owned());
    }
    out
}

/// A tile's sprite name and file: beside the properties file, or a path from the namespace's
/// folder when it names one (`optifine/ctm/glass/1.png`, `minecraft:textures/block/stone`).
fn tile_file(root: &Path, namespace: &str, properties: &Path, name: &str) -> (String, PathBuf) {
    let name = name.strip_suffix(".png").unwrap_or(name);
    let (namespace, relative) = match name.split_once(':') {
        Some((ns, path)) => (ns.to_owned(), path.to_owned()),
        None if name.contains('/') => (namespace.to_owned(), name.to_owned()),
        None => {
            let assets = root.join("assets").join(namespace);
            let dir = properties.parent().unwrap_or(&assets);
            let relative = dir
                .strip_prefix(&assets)
                .map(|d| d.join(name).to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| name.to_owned());
            (namespace.to_owned(), relative)
        }
    };
    let file = root
        .join("assets")
        .join(&namespace)
        .join(format!("{relative}.png"));
    (format!("{namespace}:{relative}"), file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_connection_has_one_tile() {
        let keys: Vec<u8> = TILES.iter().map(|t| tile_key(t)).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 47);
        // Corners only count between connected edges.
        for key in keys {
            for (bit, a, b) in CORNERS {
                assert!(key & bit == 0 || (key & a != 0 && key & b != 0), "{key:#b}");
            }
        }
        assert_eq!(TILES[0], "");
        assert_eq!(TILES[26], "URDL abcd");
    }

    #[test]
    fn properties_split_at_the_first_separator() {
        let p = properties(
            "# glass\nmatchBlocks=minecraft:oak_stairs:facing=east,west\n! x\ntiles: 0-3 \\\n  4\nmethod=ctm",
        );
        assert_eq!(p["matchBlocks"], "minecraft:oak_stairs:facing=east,west");
        assert_eq!(p["tiles"], "0-3 4");
        assert_eq!(tile_names(&p["tiles"]), ["0", "1", "2", "3", "4"]);
        let m = BlockMatch::parse(&p["matchBlocks"]).unwrap();
        assert_eq!(m.name, "minecraft:oak_stairs");
        let mut props = BTreeMap::new();
        props.insert("facing".to_owned(), "west".to_owned());
        assert!(m.matches("minecraft:oak_stairs", &props));
        props.insert("facing".to_owned(), "north".to_owned());
        assert!(!m.matches("minecraft:oak_stairs", &props));
        assert_eq!(BlockMatch::parse("glass").unwrap().name, "minecraft:glass");
    }

    #[test]
    fn texture_axes_follow_the_quads_coordinates() {
        // An up face with vanilla's coordinates: u east, v south.
        let p = [
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 1.0],
            [1.0, 1.0, 1.0],
            [1.0, 1.0, 0.0],
        ];
        let uv = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
        assert_eq!(texture_axes(Direction::Up, &p, &uv), ([1, 0, 0], [0, 0, 1]));
        // A north face: u west, v down.
        let p = [
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        assert_eq!(
            texture_axes(Direction::North, &p, &uv),
            ([-1, 0, 0], [0, -1, 0])
        );
    }

    /// A wall of glass three blocks wide and high: the middle face connects all round, a
    /// corner face on two sides, and a block of another kind connects to nothing.
    #[test]
    fn a_glass_wall_picks_its_tiles() {
        let mut ctm = Ctm {
            block_of: vec![0, 1, 1, 2],
            tile_of: vec![0; 256],
            ..Default::default()
        };
        for (index, tile) in TILES.iter().enumerate() {
            ctm.tile_of[tile_key(tile) as usize] = index as u8;
        }
        ctm.rules.push(Rule {
            tiles: (100..147).map(Tile::Layer).collect(),
            faces: 0b11_1111,
            connect: Connect::Block,
            inner_seams: false,
            sprites: Vec::new(),
        });
        // States 1 and 2 are two states of the glass block, drawn with sprite 7.
        ctm.by_state = vec![vec![], vec![(0, 7)], vec![(0, 7)], vec![]];
        let p = [
            [1.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
        ];
        let uv = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
        // A wall in the x-y plane; a north face's right is -x and down is -y.
        let wall = |[x, y, z]: [i32; 3]| -> u32 {
            if z == 0 && (-1..=1).contains(&x) && (-1..=1).contains(&y) {
                2
            } else {
                0
            }
        };
        let middle = ctm.tile(1, 7, Direction::North, &p, &uv, wall);
        assert_eq!(middle, Some(126));
        // The top-left corner of the wall seen from the north (right is -x): from the block
        // at x = 1, y = 1, right and down connect, with the corner between them.
        let corner = ctm.tile(1, 7, Direction::North, &p, &uv, |[x, y, z]| {
            wall([x + 1, y + 1, z])
        });
        assert_eq!(corner, Some(113)); // "RD c"
        // Another sprite of the block is not this rule's.
        assert_eq!(ctm.tile(1, 8, Direction::North, &p, &uv, wall), None);
        // Alone: tile 0.
        assert_eq!(ctm.tile(1, 7, Direction::North, &p, &uv, |_| 0), Some(100));
    }

    /// A rule beside its 47 tiles loads them as sprites, in order; a method Horizon does not
    /// draw is skipped.
    #[test]
    fn rules_load_their_tiles() {
        let root = std::env::temp_dir().join(format!("horizon-ctm-{}", std::process::id()));
        let dir = root.join("assets/minecraft/optifine/ctm/glass");
        std::fs::create_dir_all(&dir).unwrap();
        for tile in 0..47u8 {
            let mut encoder = png::Encoder::new(
                std::fs::File::create(dir.join(format!("{tile}.png"))).unwrap(),
                16,
                16,
            );
            encoder.set_color(png::ColorType::Rgba);
            let pixels = [tile, 0, 0, 255].repeat(256);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&pixels)
                .unwrap();
        }
        let file = dir.join("glass.properties");
        std::fs::write(
            &file,
            "matchBlocks=glass\nmethod=ctm\nfaces=sides top\ntiles=0-46",
        )
        .unwrap();
        std::fs::write(dir.join("other.properties"), "method=overlay\ntiles=0").unwrap();
        let files = rule_files(&root);
        assert_eq!(files.len(), 2);
        let mut sprites = Sprites::default();
        let rule = ParsedRule::load(&root, "minecraft", &file, &mut sprites)
            .unwrap()
            .unwrap();
        let other = ParsedRule::load(&root, "minecraft", &files[1].1, &mut sprites).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(other.is_none());
        assert_eq!(rule.tiles.len(), 47);
        assert_eq!(rule.tiles[46], Tile::Layer(46));
        assert_eq!(rule.connect, Connect::Block);
        assert_eq!(rule.faces, 0b11_1110);
        assert_eq!(rule.match_blocks[0].name, "minecraft:glass");
        assert_eq!(sprites.layers["minecraft:optifine/ctm/glass/5"], 5);
    }
}
