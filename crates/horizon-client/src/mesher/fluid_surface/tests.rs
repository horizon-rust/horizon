use super::*;
use crate::mesher::{P, Tint};
use horizon_world::fluid::FluidState;
use std::collections::HashMap;

const AIR: u32 = 0;
const STONE: u32 = 1;
const SOURCE: u32 = 2;
/// Flowing water of amount 1..=7 is `FLOWING + amount - 1`.
const FLOWING: u32 = 3;
/// Falling water (amount 8).
const FALLING: u32 = 10;
const SPRITES: Sprites = Sprites {
    water_still: 10,
    water_flow: 11,
    water_overlay: 12,
    lava_still: 13,
    lava_flow: 14,
};

fn info() -> Vec<Info> {
    let base = Info {
        solid: false,
        full: false,
        permeable: true,
        shade: 1.0,
        emission: 0,
        emissive: false,
        fluid: FluidState::EMPTY,
        model: false,
        block_entity: false,
        tints: [Tint::None; 2],
        tint_count: 0,
        offset: None,
        legacy_solid: false,
        overlay: false,
        leaves: false,
        blocks_flow: false,
        ice: false,
        sturdy: 0,
        face: [0; 6],
        cover: [0.0; 6],
        light_colour: [0; 3],
        light_at: [255; 3],
        flame: 0,
        map_color: 0,
        ray: 0,
        ray_shape: 0,
    };
    let stone = Info {
        solid: true,
        full: true,
        permeable: false,
        legacy_solid: true,
        blocks_flow: true,
        sturdy: 0b11_1111,
        face: [1; 6],
        cover: [1.0; 6],
        ..base
    };
    let water = |fluid| Info { fluid, ..base };
    let mut info = vec![base, stone, water(FluidState::WATER)];
    for amount in 1..=7 {
        info.push(water(FluidState::new(
            FluidKind::FlowingWater,
            amount,
            false,
        )));
    }
    info.push(water(FluidState::new(FluidKind::FlowingWater, 8, true)));
    info
}
fn scene(block: impl Fn(i32, i32, i32) -> u32) -> Padded {
    let mut pad = Padded {
        blocks: vec![AIR; P * P * P],
        light: vec![0xF0; P * P * P],
    };
    for y in -2..18 {
        for z in -2..18 {
            for x in -2..18 {
                pad.blocks[pi(x, y, z)] = block(x, y, z);
            }
        }
    }
    pad
}
fn mesh(pad: &Padded) -> SectionMesh {
    let info = info();
    let mut out = SectionMesh::default();
    let mut lattice = Lattice::new();
    for y in 0..16 {
        for z in 0..16 {
            for x in 0..16 {
                if info[pad.blocks[pi(x, y, z)] as usize].fluid.kind() != FluidKind::Empty {
                    let cell = [x, y, z];
                    tesselate(
                        &info,
                        SPRITES,
                        false,
                        pad,
                        0x3F76E4,
                        &mut out,
                        &mut lattice,
                        cell,
                    );
                }
            }
        }
    }
    out
}
fn quads(v: &[TerrainVertex]) -> impl Iterator<Item = &[TerrainVertex; 4]> {
    v.as_chunks::<4>().0.iter()
}
fn key(p: [f32; 3]) -> [i32; 3] {
    p.map(|c| (c * 1000.0).round() as i32)
}

/// A rolling sheet of every flowing level on a stone floor, with holes of air
/// and stone.
fn sheet(x: i32, y: i32, z: i32) -> u32 {
    let h = (x * 7 + z * 13).rem_euclid(9) as u32;
    match y {
        ..3 => STONE,
        3 if (x + 2 * z).rem_euclid(11) == 0 => STONE,
        3 if (2 * x + z).rem_euclid(13) == 0 => AIR,
        3 if h >= 8 => SOURCE,
        3 => FLOWING + h.min(6),
        _ => AIR,
    }
}

#[test]
fn neighbouring_tops_meet_at_every_corner() {
    let out = mesh(&scene(sheet));
    // Top vertices (on the upper surface, off the block grid's vertical
    // faces' bottoms) by their column: one height, light and colour each.
    let mut seen: HashMap<[i32; 2], (i32, u16, u32)> = HashMap::new();
    let mut tops = 0;
    for q in quads(&out.translucent) {
        let flat_x = q.iter().all(|v| v.pos[0] == q[0].pos[0]);
        let flat_z = q.iter().all(|v| v.pos[2] == q[0].pos[2]);
        let bottom = q.iter().all(|v| v.pos[1] == q[0].pos[1]) && q[0].pos[1] < 3.5;
        if flat_x || flat_z || bottom {
            continue;
        }
        tops += 1;
        for v in q {
            let k = key(v.pos);
            let entry = seen.entry([k[0], k[2]]).or_insert((k[1], v.light, v.color));
            assert_eq!(*entry, (k[1], v.light, v.color), "corner {:?}", v.pos);
        }
    }
    assert!(tops > 200, "{tops} top quads");
}

#[test]
fn a_sheet_pouring_over_a_ledge_has_no_step() {
    // Upper sheet on a ledge (y 5, x <= 8), a falling column at x 8 and a
    // lower sheet at y 4 beyond it.
    let pad = scene(|x, y, _| match (x, y) {
        (_, ..4) => STONE,
        (..8, 4) => STONE,
        (8, 4) => FALLING,
        (9.., 4) => FLOWING + 6 - (x - 9).clamp(0, 6) as u32,
        (..8, 5) => SOURCE,
        (8, 5) => FLOWING + 6,
        _ => AIR,
    });
    let out = mesh(&pad);
    // Every vertex on the plane x = 9 (the ledge's edge beyond the column)
    // sits at the height the lower sheet reaches there: y 5 under the epsilon.
    let mut on_edge = 0;
    for q in quads(&out.translucent) {
        let vertical = q.iter().all(|v| v.pos[0] == q[0].pos[0]);
        for v in q
            .iter()
            .filter(|v| v.pos[0] == 9.0 && v.pos[1] > 4.5 && !vertical)
        {
            assert!((v.pos[1] - (5.0 - EPSILON)).abs() < 1e-5, "{:?}", v.pos);
            on_edge += 1;
        }
        // No wall faces east at the column's top.
        let east = q.iter().all(|v| (v.pos[0] - (9.0 - EPSILON)).abs() < 1e-6);
        assert!(!(east && q[0].pos[1] > 5.0 - 2.0 * EPSILON), "{q:?}");
    }
    assert!(on_edge > 0);
}

#[test]
fn flowing_textures_continue_across_blocks() {
    // A channel of flowing water along +x between stone walls.
    let pad = scene(|x, y, z| match (y, z) {
        (..3, _) => STONE,
        (3, 4..=6) => FLOWING + (6 - (x + 2) / 3).max(0) as u32,
        (3, _) => STONE,
        _ => AIR,
    });
    let out = mesh(&pad);
    let mut uv_at: HashMap<[i32; 2], [i32; 2]> = HashMap::new();
    for q in quads(&out.translucent) {
        if q[0].layer != SPRITES.water_flow || q.iter().any(|v| v.pos[1] < 3.5) {
            continue;
        }
        for v in q {
            let k = key(v.pos);
            // Modulo whole sprites (the sampler repeats).
            let uv = v
                .uv()
                .map(|c| ((c * 1000.0).round() as i32).rem_euclid(1000));
            assert_eq!(*uv_at.entry([k[0], k[2]]).or_insert(uv), uv, "{:?}", v.pos);
        }
    }
    assert!(!uv_at.is_empty());
}

#[test]
fn flat_water_keeps_vanilla_shade_and_columns_round_their_edges() {
    // A still pool (sources over stone) and a lone column of water in the air.
    let pad = scene(|x, y, z| match (x, y, z) {
        (_, ..3, _) => STONE,
        (_, 3, _) if x < 8 => SOURCE,
        (12, 3..=8, 12) => SOURCE,
        _ => AIR,
    });
    let out = mesh(&pad);
    let white = |shade: f32| ((0x3F as f32) * shade) as u32;
    let mut column_edges = 0;
    for q in quads(&out.translucent) {
        let centre = q.iter().map(|v| v.pos[0]).sum::<f32>() / 4.0;
        let red = |v: &TerrainVertex| v.color & 0xFF;
        if centre < 6.0 && q.iter().all(|v| v.pos[1] > 3.5) {
            // The pool's interior: flat, vanilla's up shade (1.0).
            assert!(q.iter().all(|v| red(v) == white(1.0)), "{q:?}");
        }
        if (11.0..=13.0).contains(&centre) && q.iter().all(|v| v.pos[1] < 8.9) {
            let vertical = q.iter().all(|v| v.pos[0] == q[0].pos[0])
                || q.iter().all(|v| v.pos[2] == q[0].pos[2]);
            if vertical {
                // Each end of a column face is an outer edge: 0.6 and 0.8 meet at 0.7.
                for v in q {
                    assert_eq!(red(v), white(0.7), "{:?}", v.pos);
                    column_edges += 1;
                }
            }
        }
    }
    assert!(column_edges > 0);
}
