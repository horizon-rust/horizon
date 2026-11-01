//! The far tiles as a tree the GPU walks (`lod_gpu.wgsl`).
//!
//! Tiles are quadtree cells (`TileKey`), and the plan keeps only the cells it
//! draws: leaves, and for a while a replaced parent beside its children. The
//! tree adds their ancestors up to the coarsest level in use, so that one box
//! test on an ancestor leaves out everything under it, and lays the nodes out
//! breadth first: a node's children are consecutive, and each level is one
//! stretch of the array, which is the order the shader visits them in.
use crate::{fast_hash::FxHashMap, lod::TileKey};

/// No node, tile or child.
pub const NONE: u32 = u32::MAX;
/// A time that never came (`TileRecord::shown`, `dying`).
pub const NO_TIME: i32 = i32::MIN;

/// What the shader needs to draw a tile's solid quads: where they are in the
/// arena, how they are sorted by facing side (`mesher::Sides`) and when the
/// tile began to fade in or out (`gfx/lod.rs`'s `Tile`), in milliseconds on
/// the renderer's clock.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct TileRecord {
    /// First vertex of the solid quads in its arena page.
    pub base: u32,
    pub page: u32,
    pub level: u32,
    /// When it began to fade in, `NO_TIME` while its job still uploads.
    pub shown: i32,
    /// When its replacements began to fade in, or `NO_TIME`.
    pub dying: i32,
    /// 1 when it is dying with nothing to replace it (it fades out).
    pub alone: u32,
    /// `Sides::ends` and `Sides::planes`.
    pub ends: [u32; 7],
    pub planes: [f32; 7],
}

/// One node: a tile, an ancestor of tiles, or both.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Node {
    /// Origin of its cell in blocks, x then z.
    pub origin: [i32; 2],
    pub level: u32,
    /// Index of its `TileRecord`, or `NONE` for an ancestor alone.
    pub tile: u32,
    /// The box that holds it and everything under it, relative to `origin`
    /// (y absolute).
    pub min: [f32; 3],
    pub first_child: u32,
    pub max: [f32; 3],
    pub child_count: u32,
}

/// A tile with solid quads, as `build` takes it.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub key: TileKey,
    /// The tile's bounds relative to its origin.
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub record: TileRecord,
}

#[derive(Default)]
pub struct Tree {
    /// Breadth first: the `roots` nodes of the coarsest level first.
    pub nodes: Vec<Node>,
    pub tiles: Vec<TileRecord>,
    pub roots: u32,
    /// Levels from the roots to the finest tile, how many steps a walk takes.
    pub levels: u32,
    /// Tiles per arena page, which sizes each page's share of the draw list.
    pub tiles_per_page: Vec<u32>,
}

struct Work {
    key: TileKey,
    tile: u32,
    min: [f32; 3],
    max: [f32; 3],
    children: Vec<usize>,
}

impl Work {
    fn new(key: TileKey) -> Self {
        Self {
            key,
            tile: NONE,
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
            children: Vec::new(),
        }
    }
}

/// The tree of `entries`: tile `i` is `Tree::tiles[i]`.
pub fn build(entries: &[Entry]) -> Tree {
    let Some(top) = entries.iter().map(|e| e.key.level).max() else {
        return Tree::default();
    };
    let mut works: Vec<Work> = Vec::new();
    let mut index: FxHashMap<TileKey, usize> = FxHashMap::default();
    for (i, e) in entries.iter().enumerate() {
        let id = *index.entry(e.key).or_insert_with(|| {
            works.push(Work::new(e.key));
            works.len() - 1
        });
        let work = &mut works[id];
        work.tile = i as u32;
        work.min = e.min;
        work.max = e.max;
    }
    // Ancestors up to the top level, each linked to its children once.
    for e in entries {
        let mut child = e.key;
        while child.level < top {
            let parent = child.parent();
            let parent_id = *index.entry(parent).or_insert_with(|| {
                works.push(Work::new(parent));
                works.len() - 1
            });
            let child_id = index[&child];
            if works[parent_id].children.contains(&child_id) {
                break;
            }
            works[parent_id].children.push(child_id);
            child = parent;
        }
    }
    // Each node's box takes in its children's, finest level first.
    let mut order: Vec<usize> = (0..works.len()).collect();
    order.sort_by_key(|&w| works[w].key.level);
    for &w in &order {
        if works[w].key.level == top {
            continue;
        }
        let (key, min, max) = (works[w].key, works[w].min, works[w].max);
        let parent = index[&key.parent()];
        let [px, _, pz] = works[parent].key.origin();
        let [x, _, z] = key.origin();
        let offset = [(x - px) as f32, 0.0, (z - pz) as f32];
        for k in 0..3 {
            works[parent].min[k] = works[parent].min[k].min(min[k] + offset[k]);
            works[parent].max[k] = works[parent].max[k].max(max[k] + offset[k]);
        }
    }
    let keys: Vec<TileKey> = works.iter().map(|w| w.key).collect();
    for work in &mut works {
        work.children.sort_by_key(|&c| keys[c]);
    }
    let mut roots: Vec<usize> = (0..works.len())
        .filter(|&w| works[w].key.level == top)
        .collect();
    roots.sort_by_key(|&w| keys[w]);
    let root_count = roots.len() as u32;
    let mut queue = roots;
    let mut first_child = vec![0u32; works.len()];
    let mut head = 0;
    while head < queue.len() {
        let w = queue[head];
        first_child[w] = queue.len() as u32;
        queue.extend(works[w].children.iter().copied());
        head += 1;
    }
    let nodes = queue
        .iter()
        .map(|&w| {
            let work = &works[w];
            let [x, _, z] = work.key.origin();
            Node {
                origin: [x, z],
                level: u32::from(work.key.level),
                tile: work.tile,
                min: work.min,
                first_child: first_child[w],
                max: work.max,
                child_count: work.children.len() as u32,
            }
        })
        .collect();
    let mut tiles_per_page: Vec<u32> = Vec::new();
    for e in entries {
        let page = e.record.page as usize;
        if tiles_per_page.len() <= page {
            tiles_per_page.resize(page + 1, 0);
        }
        tiles_per_page[page] += 1;
    }
    Tree {
        nodes,
        tiles: entries.iter().map(|e| e.record).collect(),
        roots: root_count,
        levels: u32::from(top) + 1,
        tiles_per_page,
    }
}

/// How long a tile takes to fade, in milliseconds (`gfx/lod.rs`'s `FADE`).
pub const FADE_MS: i32 = 600;

/// `gfx/lod.rs`'s `fade_code` for a tile that began to fade in `age` ms ago:
/// the shader's `fade_code` is this, in the same arithmetic.
#[cfg(test)]
pub fn fade_code(age: i32, level: u8) -> u32 {
    let faded = (age.max(0) as f32 / FADE_MS as f32).min(2.0);
    let shown = (faded * 64.0) as u32 + 1;
    if shown >= 64 {
        0
    } else {
        1 + u32::from(level) * 129 + shown
    }
}

/// `gfx/lod.rs`'s `fade_out_code`, as the shader has it.
#[cfg(test)]
pub fn fade_out_code(age: i32, level: u8) -> u32 {
    let faded = (age.max(0) as f32 / FADE_MS as f32).min(2.0);
    1 + u32::from(level) * 129 + 64 + ((faded * 64.0) as u32 + 1).min(64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn entry(x: i32, z: i32, level: u8, page: u32) -> Entry {
        let size = TileKey { x, z, level }.size() as f32;
        Entry {
            key: TileKey { x, z, level },
            min: [0.0, 60.0, 0.0],
            max: [size, 70.0 + level as f32, size],
            record: TileRecord {
                base: x as u32 * 4,
                page,
                level: u32::from(level),
                shown: 0,
                dying: NO_TIME,
                alone: 0,
                ends: [0, 0, 0, 4, 4, 4, 4],
                planes: [0.0; 7],
            },
        }
    }

    #[test]
    fn tiles_get_ancestors_up_to_the_coarsest_level_in_use() {
        let entries = [
            entry(0, 0, 0, 0),
            entry(1, 0, 0, 0),
            entry(5, 5, 0, 1),
            entry(3, 3, 2, 1),
        ];
        let tree = build(&entries);
        assert_eq!(tree.levels, 3);
        assert_eq!(
            tree.roots, 3,
            "(0,0) and (1,1) over the fine tiles, and (3,3)"
        );
        // Level 2: three roots. Level 1: (0,0) and (2,2). Level 0: three tiles.
        assert_eq!(tree.nodes.len(), 8);
        assert!(tree.nodes[..3].iter().all(|n| n.level == 2));
        assert_eq!(tree.tiles_per_page, [2, 2]);
        // Every node's children are consecutive, one level down, inside its box.
        let mut reached = vec![false; tree.nodes.len()];
        for r in 0..tree.roots as usize {
            reached[r] = true;
        }
        for node in &tree.nodes {
            for c in node.first_child..node.first_child + node.child_count {
                let child = &tree.nodes[c as usize];
                assert_eq!(child.level + 1, node.level);
                reached[c as usize] = true;
                let cx = (child.origin[0] - node.origin[0]) as f32;
                let cz = (child.origin[1] - node.origin[1]) as f32;
                assert!(node.min[0] <= cx + child.min[0] && cx + child.max[0] <= node.max[0]);
                assert!(node.min[2] <= cz + child.min[2] && cz + child.max[2] <= node.max[2]);
                assert!(node.min[1] <= child.min[1] && child.max[1] <= node.max[1]);
            }
        }
        assert!(reached.iter().all(|&r| r), "every node is below a root");
        // Each tile is on exactly one node, and keeps its record.
        let mut tiles: Vec<u32> = tree
            .nodes
            .iter()
            .filter(|n| n.tile != NONE)
            .map(|n| n.tile)
            .collect();
        tiles.sort_unstable();
        assert_eq!(tiles, [0, 1, 2, 3]);
        for n in tree.nodes.iter().filter(|n| n.tile != NONE) {
            assert_eq!(tree.tiles[n.tile as usize].level, n.level);
        }
    }

    #[test]
    fn a_replaced_parent_is_a_tile_and_an_ancestor() {
        let entries = [entry(0, 0, 1, 0), entry(0, 0, 0, 0), entry(1, 1, 0, 0)];
        let tree = build(&entries);
        assert_eq!(tree.roots, 1);
        assert_eq!(tree.nodes.len(), 3);
        let root = tree.nodes[0];
        assert_eq!((root.tile, root.child_count, root.first_child), (0, 2, 1));
    }

    #[test]
    fn no_tiles_make_no_tree() {
        let tree = build(&[]);
        assert!(tree.nodes.is_empty() && tree.tiles.is_empty());
        assert_eq!((tree.roots, tree.levels), (0, 0));
    }

    #[test]
    fn fades_follow_the_cpu_codes() {
        let start = Instant::now();
        for level in [0u8, 3, 10] {
            // Not where a code steps: the CPU's seconds in f32 may land either side.
            for ms in (0..=700).step_by(7).filter(|ms| ms * 8 % 75 != 0) {
                let now = start + Duration::from_millis(ms as u64);
                let cpu = super::super::super::lod::fade_code(Some(start), level, now);
                let gpu = fade_code(ms, level);
                assert_eq!(cpu, Some(gpu), "in {ms} ms at level {level}");
                assert_eq!(
                    super::super::super::lod::fade_out_code(start, level, now),
                    fade_out_code(ms, level),
                    "out {ms} ms at level {level}"
                );
            }
        }
    }
}
