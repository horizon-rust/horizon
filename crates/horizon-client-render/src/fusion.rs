//! Fusion connected textures at mesh time. A pack's connecting textures (a `fusion` section in
//! their `.mcmeta`) are cut into a sprite per tile at load; each block state whose model is a
//! `fusion:connecting` one gets, for each such texture on its faces, the predicates it joins by
//! with their blocks resolved to state ids. Meshing a face then evaluates them in the face's
//! eight directions and draws the layout's tiles through [`crate::connected`]'s [`Pick`] and
//! [`Overlay`]. Without Fusion files nothing is loaded and nothing changes.
//!
//! The format is `horizon_optifine::fusion`'s.

use crate::connected::{Overlay, Pick, TileLayer};
use crate::sprites::Sprites;
use horizon_optifine::cem::json::Json;
use horizon_optifine::ctm::connect::{self, Axes, Face};
use horizon_optifine::fusion::{
    Connection, Layout, Tiles,
    model::{Connections, ModelType},
    predicate::Query,
    texture::{Kind, Meta},
};
use horizon_world::block_state_properties::BlockProperties;
use horizon_world::registry::{BlockRegistry, BlockStateId};
use std::{collections::HashMap, fs, path::Path};

/// A connecting texture as drawn: its layout and its tiles, `None` for an empty one.
struct Sheet {
    layout: Layout,
    tiles: Vec<Option<TileLayer>>,
    /// The texture's own predicates, for models that name none.
    connections: Connection,
}

/// A sheet on a state's faces and what it joins by, blocks as sorted state ids.
struct Rule {
    sheet: usize,
    connection: Connection<Vec<u32>>,
}

#[derive(Default)]
pub struct FusionTextures {
    sheets: Vec<Sheet>,
    rules: Vec<Rule>,
    /// By block state: a range of `faces`, each a sprite layer its faces show (the whole
    /// sheet) and the rule for it.
    by_state: Vec<(u32, u32)>,
    faces: Vec<(u32, u32)>,
    /// Each state's block, for `is_same_block`.
    blocks: Vec<u32>,
}

/// The sprite name of a model's texture reference (`block/glass`, `ns:block/x`).
fn sprite(reference: &str) -> String {
    horizon_optifine::matchers::namespaced(reference)
}

fn read_json(path: &Path) -> Option<Json> {
    Json::parse(&fs::read_to_string(path).ok()?).ok()
}

/// A model's file and its parents' (up to one that has no file, `builtin/generated`), the
/// model first; empty when the model has no file.
fn model_chain(root: &Path, name: &str) -> Vec<Json> {
    let mut chain = Vec::new();
    let mut next = Some(sprite(name));
    // Parents deeper than vanilla's few levels are a loop.
    while let Some(name) = next.take().filter(|_| chain.len() < 16) {
        let Some((ns, path)) = name.split_once(':') else {
            break;
        };
        let Some(model) = read_json(&root.join(format!("assets/{ns}/models/{path}.json"))) else {
            break;
        };
        next = model.get("parent").and_then(Json::as_str).map(sprite);
        chain.push(model);
    }
    chain
}

/// Of a `fusion:connecting` model: each texture key's sprite and the model's predicates for
/// it (`None`: the texture's own). Faces' own `connections` references aren't read: a face
/// joins by its texture's key.
fn connecting_model(root: &Path, name: &str) -> Vec<(String, Option<Connection>)> {
    let chain = model_chain(root, name);
    if chain.first().and_then(ModelType::of) != Some(ModelType::Connecting) {
        return Vec::new();
    }
    // Parents first, so a child's keys replace theirs.
    let mut connections = Connections::Textures;
    let mut textures: HashMap<&str, &str> = HashMap::new();
    for model in chain.iter().rev() {
        match Connections::of(model) {
            Ok(own) => connections = own.inherit(&connections),
            Err(e) => log::warn!("Horizon: Fusion model {name}: {e}"),
        }
        for (key, value) in model
            .get("textures")
            .and_then(Json::as_object)
            .unwrap_or(&[])
        {
            if let Some(value) = value.as_str() {
                textures.insert(key.as_str(), value);
            }
        }
    }
    textures
        .iter()
        .filter_map(|(&key, &value)| {
            // `#other` references, as vanilla resolves them.
            let mut value = value;
            for _ in 0..16 {
                match value.strip_prefix('#') {
                    Some(other) => value = *textures.get(other)?,
                    None => break,
                }
            }
            (!value.starts_with('#'))
                .then(|| (sprite(value), connections.for_face(None, key).cloned()))
        })
        .collect()
}

/// The connecting textures under `root`: sprite names and their metadata.
fn connecting_textures(root: &Path) -> Vec<(String, Meta)> {
    let mut out = Vec::new();
    let Ok(namespaces) = fs::read_dir(root.join("assets")) else {
        return out;
    };
    for ns in namespaces.flatten() {
        let ns_name = ns.file_name().to_string_lossy().into_owned();
        let textures = ns.path().join("textures");
        let mut dirs = vec![textures.clone()];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                let Some(png) = path.to_str().and_then(|p| p.strip_suffix(".png.mcmeta")) else {
                    continue;
                };
                let Ok(text) = fs::read_to_string(&path) else {
                    continue;
                };
                if !text.contains("fusion") {
                    continue;
                }
                let Some(rel) = Path::new(png)
                    .strip_prefix(&textures)
                    .ok()
                    .and_then(|p| p.to_str())
                else {
                    continue;
                };
                let name = format!("{ns_name}:{}", rel.replace('\\', "/"));
                match Meta::from_mcmeta(&text) {
                    Ok(Some(meta)) => out.push((name, meta)),
                    Ok(None) => {}
                    Err(e) => log::warn!("Horizon: Fusion texture {name}: {e}"),
                }
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

impl FusionTextures {
    /// The connecting textures under `root` with their tiles inserted into `sprites` (before
    /// it finishes). `models` answers a block state's model names.
    pub fn load(
        root: &Path,
        registry: &BlockRegistry,
        sprites: &mut Sprites,
        mut models: impl FnMut(BlockStateId) -> Vec<String>,
    ) -> Self {
        let mut sheets = Vec::new();
        // The layer each sheet shows as on the models' faces, and the sheets by name.
        let mut layers = Vec::new();
        let mut by_name = HashMap::new();
        for (name, meta) in connecting_textures(root) {
            let Kind::Connecting {
                layout,
                connections,
                per_tile_animation,
                ..
            } = meta.kind
            else {
                continue;
            };
            let Some(&layer) = sprites.layers.get(&name) else {
                continue;
            };
            match sprites.insert_tiles(root, &name, layout.grid(), per_tile_animation) {
                Ok(tiles) => {
                    let tiles = tiles
                        .into_iter()
                        .map(|t| {
                            t.and_then(|l| {
                                Some(TileLayer::Layer {
                                    layer: u16::try_from(l).ok()?,
                                    translucent: sprites.translucent[l as usize],
                                    opaque: sprites.opaque[l as usize],
                                })
                            })
                        })
                        .collect();
                    layers.push(layer);
                    by_name.insert(name, sheets.len());
                    sheets.push(Sheet {
                        layout,
                        tiles,
                        connections,
                    });
                }
                Err(e) => log::warn!("Horizon: Fusion texture {name}: {e}"),
            }
        }
        if sheets.is_empty() {
            return Self::default();
        }
        let states: Vec<_> = (0..registry.len() as u32)
            .map(|id| {
                registry
                    .state(BlockStateId(id))
                    .map(|s| (s.name.as_str(), &s.properties))
            })
            .collect();
        // Each state's (sheet, the model's predicates for it).
        let mut cache: HashMap<String, Vec<(String, Option<Connection>)>> = HashMap::new();
        let used = (0..states.len() as u32).map(|id| {
            let mut out = Vec::new();
            for model in models(BlockStateId(id)) {
                let rules = cache
                    .entry(model)
                    .or_insert_with_key(|name| connecting_model(root, name));
                for (sprite, connection) in rules.iter() {
                    if let Some(&sheet) = by_name.get(sprite) {
                        out.push((sheet, connection.clone()));
                    }
                }
            }
            out
        });
        let used: Vec<_> = used.collect();
        Self::build(sheets, &layers, states, used)
    }

    /// The lookups for `sheets` (shown as `layers` on faces) over every block state (`None`
    /// for an id the registry skips), each using the sheets in `used`.
    fn build(
        sheets: Vec<Sheet>,
        layers: &[u32],
        states: Vec<Option<(&str, &BlockProperties)>>,
        used: Vec<Vec<(usize, Option<Connection>)>>,
    ) -> Self {
        let mut out = Self::default();
        let mut block_states: HashMap<&str, Vec<u32>> = HashMap::new();
        let mut blocks = HashMap::new();
        for (id, state) in states.iter().enumerate() {
            let block = match state {
                Some((name, _)) => {
                    block_states.entry(*name).or_default().push(id as u32);
                    let next = blocks.len() as u32;
                    *blocks.entry(*name).or_insert(next)
                }
                None => u32::MAX,
            };
            out.blocks.push(block);
        }
        // Block lists to the states they match.
        let mut resolve = |list: Vec<horizon_optifine::BlockMatch>| {
            let mut ids: Vec<u32> = list
                .iter()
                .flat_map(|m| {
                    let candidates = block_states
                        .get(m.id.as_str())
                        .map_or(&[][..], Vec::as_slice);
                    candidates.iter().copied().filter(|&id| {
                        states[id as usize].is_some_and(|(name, props)| {
                            m.matches(name, |k| props.get(k).map(String::as_str))
                        })
                    })
                })
                .collect();
            ids.sort_unstable();
            ids.dedup();
            ids
        };
        // One rule per sheet and predicate (as written), shared by the states using them.
        let mut written: Vec<(usize, Connection)> = Vec::new();
        for list in used {
            let start = out.faces.len() as u32;
            for (sheet, connection) in list {
                let connection = connection.unwrap_or_else(|| sheets[sheet].connections.clone());
                let found = written
                    .iter()
                    .position(|w| w.0 == sheet && w.1 == connection);
                let rule = match found {
                    Some(rule) => rule as u32,
                    None => {
                        written.push((sheet, connection.clone()));
                        out.rules.push(Rule {
                            sheet,
                            connection: connection.map_blocks(&mut resolve),
                        });
                        out.rules.len() as u32 - 1
                    }
                };
                if !out.faces[start as usize..]
                    .iter()
                    .any(|f| f.0 == layers[sheet])
                {
                    out.faces.push((layers[sheet], rule));
                }
            }
            out.by_state.push((start, out.faces.len() as u32 - start));
        }
        out.sheets = sheets;
        out
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// What the `face` of the block `state` draws for a quad showing sprite `layer`: `None`
    /// when Fusion has nothing for it. `neighbour` answers the state at an offset from the
    /// block, `covers` whether a state hides the face of a block beside it; the overlay
    /// layout's pieces go to `overlay`.
    pub fn pick(
        &self,
        state: u32,
        layer: u32,
        face: Face,
        neighbour: impl Fn([i32; 3]) -> u32,
        covers: impl Fn(u32) -> bool,
        mut overlay: impl FnMut(Overlay),
    ) -> Option<Pick> {
        let &(start, len) = self.by_state.get(state as usize)?;
        let faces = &self.faces[start as usize..(start + len) as usize];
        let &(_, rule) = faces.iter().find(|f| f.0 == layer)?;
        let rule = &self.rules[rule as usize];
        let normal = face.normal();
        let offsets = connect::offsets(Axes::cube(face));
        let block = self.blocks.get(state as usize);
        let mask = rule.connection.mask(|bit, query| {
            let &(_, [x, y, z]) = offsets.iter().find(|o| o.0 == bit).expect("a mask bit");
            let in_front = || neighbour([x + normal[0], y + normal[1], z + normal[2]]);
            match query {
                Query::SameBlock => self.blocks.get(neighbour([x, y, z]) as usize) == block,
                Query::SameState => neighbour([x, y, z]) == state,
                Query::Neighbour(ids) => ids.binary_search(&neighbour([x, y, z])).is_ok(),
                Query::InFront(ids) => ids.binary_search(&in_front()).is_ok(),
                Query::FaceVisible => !covers(in_front()),
            }
        });
        let sheet = &self.sheets[rule.sheet];
        let tile = |i: u8| sheet.tiles.get(usize::from(i)).copied().flatten();
        // An empty tile draws nothing.
        Some(match sheet.layout.tiles(mask) {
            Tiles::One(t) => tile(t).map_or(Pick::Nothing, Pick::Tile),
            Tiles::Quarters(q) => match q.map(tile) {
                [Some(a), Some(b), Some(c), Some(d)] => Pick::Quarters([a, b, c, d]),
                _ => Pick::Nothing,
            },
            Tiles::Overlay(mask) => {
                for t in Layout::overlay_tiles(mask) {
                    if let Some(TileLayer::Layer {
                        layer, translucent, ..
                    }) = tile(t)
                    {
                        overlay(Overlay {
                            layer,
                            translucent,
                            tint: None,
                        });
                    }
                }
                Pick::Nothing
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tile `i` of a sheet is layer `base + i`; `empty` tiles have none.
    fn sheet(layout: Layout, base: u16, empty: &[u16], connections: Connection) -> Sheet {
        let (w, h) = layout.grid();
        Sheet {
            layout,
            tiles: (0..(w * h) as u16)
                .map(|i| {
                    (!empty.contains(&i)).then_some(TileLayer::Layer {
                        layer: base + i,
                        translucent: false,
                        opaque: false,
                    })
                })
                .collect(),
            connections,
        }
    }

    fn predicate(json: &str) -> Connection {
        Connection::parse(&Json::parse(json).unwrap()).unwrap()
    }

    fn layer(layer: u16) -> TileLayer {
        TileLayer::Layer {
            layer,
            translucent: false,
            opaque: false,
        }
    }

    /// State 0 air, 1 and 2 glass (two states), 3 stone, 4 sand. Glass shows a pieced sheet
    /// (layer 50, tiles from 100) that joins the same block; stone an overlay sheet (layer 60,
    /// tiles from 200) toward visible sand.
    fn world() -> FusionTextures {
        let none = BlockProperties::default();
        let names = [
            "minecraft:air",
            "minecraft:glass",
            "minecraft:glass",
            "minecraft:stone",
            "minecraft:sand",
        ];
        let states = names.iter().map(|n| Some((*n, &none))).collect();
        let sheets = vec![
            sheet(Layout::Pieced, 100, &[], Connection::SameBlock),
            sheet(Layout::Overlay, 200, &[7], Connection::SameState),
        ];
        let sand = predicate(
            r#"{"type": "and", "predicates": [{"type": "match_block", "block": "sand"}, {"type": "is_face_visible"}]}"#,
        );
        let used = vec![
            vec![],
            vec![(0, None)],
            vec![(0, None)],
            vec![(1, Some(sand))],
            vec![],
        ];
        FusionTextures::build(sheets, &[50, 60], states, used)
    }

    /// `pick` for the block at the origin among `blocks` (offset, state), the opaque ones
    /// in `opaque`; with the overlays it hands out.
    fn pick_in(
        fusion: &FusionTextures,
        state: u32,
        layer: u32,
        face: Face,
        blocks: &[([i32; 3], u32)],
        opaque: &[u32],
    ) -> (Option<Pick>, Vec<u16>) {
        let at = |o: [i32; 3]| blocks.iter().find(|(p, _)| *p == o).map_or(0, |b| b.1);
        let mut overlays = Vec::new();
        let pick = fusion.pick(
            state,
            layer,
            face,
            at,
            |s| opaque.contains(&s),
            |o| overlays.push(o.layer),
        );
        (pick, overlays)
    }

    #[test]
    fn glass_pieces_join_the_same_block() {
        let fusion = world();
        // Alone: every quarter the lone tile.
        assert_eq!(
            pick_in(&fusion, 1, 50, Face::North, &[], &[]).0,
            Some(Pick::Quarters([layer(100); 4]))
        );
        // A north face's right is west: glass there (in its other state) joins the right
        // quarters, the horizontal piece.
        assert_eq!(
            pick_in(&fusion, 1, 50, Face::North, &[([-1, 0, 0], 2)], &[]).0,
            Some(Pick::Quarters([
                layer(100),
                layer(103),
                layer(100),
                layer(103)
            ]))
        );
        // Stone beside it doesn't join; a face showing another sprite isn't Fusion's.
        assert_eq!(
            pick_in(&fusion, 1, 50, Face::North, &[([-1, 0, 0], 3)], &[]).0,
            Some(Pick::Quarters([layer(100); 4]))
        );
        assert_eq!(pick_in(&fusion, 1, 51, Face::North, &[], &[]).0, None);
        assert_eq!(pick_in(&fusion, 4, 50, Face::North, &[], &[]).0, None);
    }

    #[test]
    fn overlays_reach_toward_visible_sand() {
        let fusion = world();
        // Sand to the north of stone's top face, its own top open: the top edge piece, (1, 2),
        // over a face that draws nothing itself.
        let north = [([0, 0, -1], 4)];
        assert_eq!(
            pick_in(&fusion, 3, 60, Face::Up, &north, &[]),
            (Some(Pick::Nothing), vec![213])
        );
        // Stone over that sand hides its top: no edge.
        let covered = [([0, 0, -1], 4), ([0, 1, -1], 3)];
        assert_eq!(
            pick_in(&fusion, 3, 60, Face::Up, &covered, &[3]),
            (Some(Pick::Nothing), vec![])
        );
        // Sand all round: the ring, (4, 1).
        let ring: Vec<_> = [-1, 0, 1]
            .into_iter()
            .flat_map(|x| [-1, 0, 1].map(|z| ([x, 0, z], 4)))
            .filter(|(o, _)| *o != [0, 0, 0])
            .collect();
        assert_eq!(pick_in(&fusion, 3, 60, Face::Up, &ring, &[]).1, [210]);
    }

    #[test]
    fn in_front_and_empty_tiles() {
        let none = BlockProperties::default();
        let names = ["minecraft:air", "minecraft:dirt", "minecraft:sand"];
        let states = names.iter().map(|n| Some((*n, &none))).collect();
        // Dirt's side joins where sand sits in front of the neighbour (on top of the block
        // beside it, for a top face); its full sheet leaves the lone tile empty.
        let front = predicate(r#"{"type": "match_block_in_front", "block": "sand"}"#);
        let fusion = FusionTextures::build(
            vec![sheet(Layout::Full, 300, &[0], Connection::SameState)],
            &[70],
            states,
            vec![vec![], vec![(0, Some(front))], vec![]],
        );
        // Sand above the block east of the top face: joined right, the full template's (1, 0).
        let east_top = [([1, 1, 0], 2)];
        assert_eq!(
            pick_in(&fusion, 1, 70, Face::Up, &east_top, &[]).0,
            Some(Pick::Tile(layer(301)))
        );
        // Sand beside it, not in front of anything: alone, and the lone tile is empty.
        assert_eq!(
            pick_in(&fusion, 1, 70, Face::Up, &[([1, 0, 0], 2)], &[]).0,
            Some(Pick::Nothing)
        );
    }
}
