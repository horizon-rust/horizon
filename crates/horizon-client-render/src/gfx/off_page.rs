//! `move_off_page`, moved out of `gfx.rs`; the hardware rays' rows move with the ranges.
use super::*;

/// `Renderer::compact_arena`'s and `shrink_translucent`'s moves: what the meshes
/// hold on `page` of `arena` (`part`) goes to its other pages in video memory,
/// until the next range would pass `budget` bytes (it keeps what is left; the
/// first range moves whatever its size, so none keeps the page closed), the
/// copies into `encoder`, the ranges' flat draw ids (`flat`) and, in the
/// section arena, GPU culling's records (`records`) and the hardware rays' rows
/// (`rays`, by the meshes' keys: none for far tiles, which the rays don't hold)
/// along. False when the other pages have no room left.
pub(super) fn move_off_page<'a>(
    arena: &mut arena::Arena,
    encoder: &mut wgpu::CommandEncoder,
    mut flat: Option<(&mut flat::Flat, &wgpu::Device, &mut upload::Uploads)>,
    mut records: Option<&mut gpu_culling::Records>,
    mut rays: Option<&mut hw_rays::HwRays>,
    meshes: impl Iterator<Item = (Option<SectionKey>, &'a mut SectionGpu)>,
    page: u32,
    part: Part,
    budget: &mut u64,
) -> bool {
    debug_assert!(
        records.is_none() || matches!(part, Part::Section { .. }),
        "records follow the section arena's ranges"
    );
    debug_assert!(
        rays.is_none() || matches!(part, Part::Section { .. }),
        "the rays read the section arena alone"
    );
    let mut moved = false;
    for (key, s) in meshes {
        // Nothing of a mesh here: split, one without solid quads (its (0, 0) is no
        // range of page 0); in the other arenas, one without translucent quads.
        let Some((_, start, vertices)) = part.of(s).filter(|r| r.0 == page) else {
            continue;
        };
        let bytes = vertices * arena.stride();
        if moved && bytes > *budget {
            break;
        }
        let Some(to) = arena.alloc_moved(vertices) else {
            return false;
        };
        arena.copy(encoder, (page, start), to, vertices);
        arena.release(page, start, vertices);
        if let (Some((flat, device, uploads)), Some(id)) = (&mut flat, s.flat_id) {
            let key = match part {
                Part::Translucent => to.0 + flat::TRANSLUCENT_KEYS,
                _ => to.0,
            };
            flat.place(device, encoder, uploads, id, key, to.1, vertices);
        }
        part.set(s, to);
        // Its solid quads start the range in either layout.
        if let (Part::Section { .. }, Some(records), Some(id)) =
            (part, records.as_deref_mut(), s.gpu_id)
        {
            records.relocate(id, to.0, u64::from(s.solid_base()));
        }
        // The rays' row reads its quads' texels where they are now (its structure keeps
        // the geometry it was built with): left at the old range, which another mesh
        // takes, they alpha-tested and coloured with that mesh's vertices.
        if let (Part::Section { .. }, Some(rays), Some(key)) = (part, rays.as_deref_mut(), key) {
            rays.relocate(key, (page, start), to);
        }
        *budget = budget.saturating_sub(bytes);
        moved = true;
    }
    true
}
/// The meshes `move_off_page` moves: the sections by their keys, then the far
/// tiles' (no key).
pub(super) fn keyed_meshes<'a>(
    sections: &'a mut resident::Resident<SectionGpu>,
    lods: &'a mut FxHashMap<crate::lod::TileKey, lod::Tile>,
) -> impl Iterator<Item = (Option<SectionKey>, &'a mut SectionGpu)> {
    sections.iter_mut().map(|(key, s, _)| (Some(key), s)).chain(
        lods.values_mut()
            .flat_map(|tile| tile.meshes_mut())
            .map(|s| (None, s)),
    )
}
