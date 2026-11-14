//! Complete registry-bound interaction shapes for ray-face overrides.
use crate::{
    collision::CachedCollisionRegistry,
    compression::invalid,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::geometry::VoxelShape;
use std::io;
/// The pinned implementation audit establishes that every interaction shape is
/// static per state. The import retains native alias/canonical shape identity.
pub struct InteractionShapes<'r> {
    shapes: CachedCollisionRegistry<'r>,
}
impl<'r> InteractionShapes<'r> {
    pub fn from_nbt(tag: &Tag, registry: &'r BlockRegistry) -> io::Result<Self> {
        if tag.get("kind") != Some(&Tag::String("interaction".into())) {
            return Err(invalid("not an interaction shape table"));
        }
        let shapes = CachedCollisionRegistry::from_nbt(tag, registry)?;
        if !shapes.has_canonical_identities() || shapes.cached_state_count() != registry.len() {
            return Err(invalid(
                "interaction shape table must bind every state with canonical identities",
            ));
        }
        Ok(Self { shapes })
    }
    pub fn registry(&self) -> &'r BlockRegistry {
        self.shapes.registry()
    }
    pub fn shape(&self, state: BlockStateId) -> io::Result<&VoxelShape> {
        self.shapes
            .cached_shape(state)?
            .ok_or_else(|| invalid("missing interaction shape"))
    }
}

/// `BlockGetter.clipWithInteractionOverride`: retain the outline's point and inside
/// flag; only an earlier interaction entry changes its face. An interaction cannot
/// create a target when the outline misses. Both inputs are decomposed local boxes.
pub fn clip_with_interaction_override<I, J>(
    outline: I,
    interaction: J,
    start: horizon_core::geometry::Vec3,
    end: horizon_core::geometry::Vec3,
    block: horizon_core::position::BlockPos,
) -> Option<horizon_core::geometry::BlockHit>
where
    I: Iterator<Item = horizon_core::geometry::Aabb> + Clone,
    J: Iterator<Item = horizon_core::geometry::Aabb> + Clone,
{
    let mut hit = VoxelShape::clip_boxes(outline, start, end, block)?;
    if let Some(other) = VoxelShape::clip_boxes(interaction, start, end, block)
        && other.location.distance_squared(start) < hit.location.distance_squared(start)
    {
        hit.direction = other.direction;
    }
    Some(hit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::{
        geometry::{Aabb, Direction, Vec3},
        position::BlockPos,
    };

    #[test]
    fn a_ray_starting_inside_a_sapling_shape_hits_inside_opposite_look() {
        let sapling = Aabb::new(Vec3::new(0.125, 0.0, 0.125), Vec3::new(0.875, 0.75, 0.875));
        let start = Vec3::new(0.5, 200.07625, 0.5);
        let look = crate::view_direction::view_vector(40.0, -180.0);
        let end = start.add_vector(look.scale(4.5));
        let block = BlockPos::new(0, 200, 0);
        let hit = clip_with_interaction_override(
            [sapling].into_iter(),
            [].into_iter(),
            start,
            end,
            block,
        )
        .expect("hit inside sapling outline");
        assert!(hit.inside);
        assert_eq!(hit.direction, Direction::South);
        assert_eq!(hit.block, block);
    }
}
