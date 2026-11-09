use crate::{blocks::Fluid, collisions::SceneCollisions};
use horizon_core::{
    geometry::{Aabb, Axis, Vec3, VoxelShape},
    position::{BlockPos, ChunkPos},
};
use horizon_world::{
    block_collision::BlockCollisionRegistry,
    block_collision_scan::{BlockCollisionWorld, CollisionBlock},
    collision_context::CollisionContext,
    entity_obstruction::EntityIdentity,
    registry::BlockStateId,
    world_movement::{EntityMovementWorld, MovementRequest, resolve_world_movement},
};

use super::ParticleLevel;

pub(super) fn section_blocks_all_empty(
    binding: &BlockCollisionRegistry<'_>,
    chunk: &crate::chunk::Chunk,
    section_y: i32,
) -> bool {
    let blocks = usize::try_from(section_y - (chunk.min_y() >> 4))
        .ok()
        .and_then(|s| chunk.sections.get(s))
        .map(|section| &section.blocks);
    blocks.is_some_and(|blocks| {
        !blocks.is_global()
            && !blocks.maybe_has(|v| {
                binding
                    .collision_block(BlockStateId(v))
                    .map_or(true, |block| !block.empty_shape)
            })
    })
}

pub(super) fn section_reaches_out(
    binding: &BlockCollisionRegistry<'_>,
    chunk: &crate::chunk::Chunk,
    section_y: i32,
) -> bool {
    let blocks = usize::try_from(section_y - (chunk.min_y() >> 4))
        .ok()
        .and_then(|s| chunk.sections.get(s))
        .map(|section| &section.blocks);
    blocks.is_none_or(|blocks| {
        blocks.is_global() || blocks.maybe_has(|v| binding.reaches_out(BlockStateId(v)))
    })
}

pub(super) struct CachedCollisionWorld<'a, 'w, 'r> {
    pub(super) scene: &'a mut SceneCollisions<'w, 'r>,
    pub(super) cached: &'a mut Option<((i32, i32), Option<&'w crate::chunk::Chunk>)>,
}

impl<'w> BlockCollisionWorld for CachedCollisionWorld<'_, 'w, '_> {
    type Error = std::io::Error;
    type Chunk = &'w crate::chunk::Chunk;

    fn collision_chunk(&mut self, p: ChunkPos) -> std::io::Result<Option<Self::Chunk>> {
        let chunk = match *self.cached {
            Some(((cx, cz), c)) if cx == p.x && cz == p.z => c,
            _ => {
                let c = self.scene.world.chunks.get(&(p.x, p.z)).map(|c| c.as_ref());
                *self.cached = Some(((p.x, p.z), c));
                c
            }
        };
        Ok(chunk)
    }

    fn collision_block(
        &mut self,
        chunk: Self::Chunk,
        p: BlockPos,
    ) -> std::io::Result<CollisionBlock> {
        self.scene.collision_block(chunk, p)
    }

    fn may_reach_out(&mut self, chunk: Self::Chunk, section_y: i32) -> std::io::Result<bool> {
        self.scene.may_reach_out(chunk, section_y)
    }

    fn section_is_empty(&mut self, chunk: Self::Chunk, section_y: i32) -> std::io::Result<bool> {
        self.scene.section_is_empty(chunk, section_y)
    }

    fn missing_chunks_stay_missing(&self) -> bool {
        self.scene.missing_chunks_stay_missing()
    }

    fn is_suffocating(
        &mut self,
        chunk: Self::Chunk,
        block: CollisionBlock,
        p: BlockPos,
    ) -> std::io::Result<bool> {
        self.scene.is_suffocating(chunk, block, p)
    }

    fn context_shape(
        &mut self,
        block: CollisionBlock,
        p: BlockPos,
        context: &CollisionContext<'_>,
    ) -> std::io::Result<VoxelShape> {
        self.scene.context_shape(block, p, context)
    }
}

impl EntityMovementWorld for CachedCollisionWorld<'_, '_, '_> {
    type Error = std::io::Error;
    fn entity_collisions(
        &mut self,
        _entity: EntityIdentity,
        _box_: Aabb,
    ) -> std::io::Result<Vec<VoxelShape>> {
        Ok(Vec::new())
    }

    fn movement_border(
        &mut self,
        _entity: EntityIdentity,
        _box_: Aabb,
    ) -> std::io::Result<Option<VoxelShape>> {
        Ok(None)
    }
}

impl ParticleLevel<'_> {
    #[inline]
    pub fn cached_block<'b>(
        &'b self,
        cached: &mut Option<((i32, i32), Option<&'b crate::chunk::Chunk>)>,
        x: i32,
        y: i32,
        z: i32,
    ) -> BlockStateId {
        let (cx, cz) = (x >> 4, z >> 4);
        let chunk = match *cached {
            Some(((ccx, ccz), c)) if ccx == cx && ccz == cz => c,
            _ => {
                let c = self.world.chunks.get(&(cx, cz)).map(|c| c.as_ref());
                *cached = Some(((cx, cz), c));
                c
            }
        };
        chunk.map_or(self.world.air, |c| c.block(x & 15, y, z & 15))
    }

    pub(crate) fn collide_cached<'b>(
        &'b self,
        cached: &mut Option<((i32, i32), Option<&'b crate::chunk::Chunk>)>,
        bb: Aabb,
        d: [f64; 3],
    ) -> [f64; 3] {
        let movement = Vec3::new(d[0], d[1], d[2]);
        // `Entity.collideBoundingBox(null, ...)`: no entity shapes, no border. Where no
        // block of the swept box's cells can collide, it finds no shape and the
        // movement stands, as the resolution below would return it.
        if self.cells_without_collision_cached(cached, bb.expand_towards(movement)) {
            return d;
        }
        let mut scene = SceneCollisions {
            world: self.world,
            binding: self.collision,
            subject: None,
        };
        let mut cached_world = CachedCollisionWorld {
            scene: &mut scene,
            cached,
        };
        match resolve_world_movement(
            &mut cached_world,
            MovementRequest {
                entity: EntityIdentity(0),
                context: CollisionContext::position(bb.min_position().y),
                bounds: bb,
                movement,
                max_step: 0.0,
                on_ground: false,
                candidate_budget: 1_000_000,
            },
        ) {
            Ok(v) => [v.x, v.y, v.z],
            Err(_) => d,
        }
    }

    pub(crate) fn cells_without_collision_cached<'b>(
        &'b self,
        cached: &mut Option<((i32, i32), Option<&'b crate::chunk::Chunk>)>,
        bounds: Aabb,
    ) -> bool {
        let (min, max) = (bounds.min_position(), bounds.max_position());
        if !min.is_finite() || !max.is_finite() {
            return false;
        }
        // The scan's cells: one past the box on every side (`block_collision_scan::cells`).
        let lo = BlockPos::containing(Vec3::new(min.x - 1.0e-7, min.y - 1.0e-7, min.z - 1.0e-7))
            .offset(-1, -1, -1);
        let hi = BlockPos::containing(Vec3::new(max.x + 1.0e-7, max.y + 1.0e-7, max.z + 1.0e-7))
            .offset(1, 1, 1);
        let size = |a: i32, b: i32| i64::from(b) - i64::from(a) + 1;
        let cells = size(lo.x, hi.x) * size(lo.y, hi.y) * size(lo.z, hi.z);
        if lo.x > hi.x || lo.y > hi.y || lo.z > hi.z || cells > 4096 {
            return false;
        }

        let min_section = lo.y >> 4;
        let max_section = hi.y >> 4;
        let min_cx = lo.x >> 4;
        let max_cx = hi.x >> 4;
        let min_cz = lo.z >> 4;
        let max_cz = hi.z >> 4;

        for cz in min_cz..=max_cz {
            for cx in min_cx..=max_cx {
                let chunk = match *cached {
                    Some(((cached_cx, cached_cz), c)) if cached_cx == cx && cached_cz == cz => c,
                    _ => {
                        let c = self.world.chunks.get(&(cx, cz)).map(|c| c.as_ref());
                        *cached = Some(((cx, cz), c));
                        c
                    }
                };
                let Some(chunk) = chunk else {
                    return false;
                };
                if chunk.sections.is_empty() {
                    continue;
                }
                for section_y in min_section..=max_section {
                    if !section_blocks_all_empty(self.collision, chunk, section_y) {
                        return false;
                    }
                    if (section_y > min_section || section_y < max_section)
                        && section_reaches_out(self.collision, chunk, section_y)
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    /// The fluid at a block and `FluidState.getHeight`.
    #[inline]
    pub(crate) fn fluid_cached<'b>(
        &'b self,
        cached: &mut Option<((i32, i32), Option<&'b crate::chunk::Chunk>)>,
        b: [i32; 3],
    ) -> (Fluid, f64) {
        let s = self.facts.get(self.cached_block(cached, b[0], b[1], b[2]));
        if s.fluid == Fluid::None {
            return (Fluid::None, 0.0);
        }
        let above = self
            .facts
            .get(self.cached_block(cached, b[0], b[1] + 1, b[2]));
        let height = if above.fluid == s.fluid {
            1.0
        } else {
            f64::from(s.fluid_level) / 9.0
        };
        (s.fluid, height)
    }

    /// The fluid at a block and whether it is a source (`FluidState.isSource`).
    #[inline]
    pub(crate) fn fluid_source_cached<'b>(
        &'b self,
        cached: &mut Option<((i32, i32), Option<&'b crate::chunk::Chunk>)>,
        b: [i32; 3],
    ) -> (Fluid, bool) {
        let s = self.facts.get(self.cached_block(cached, b[0], b[1], b[2]));
        (s.fluid, s.fluid_level == 8 && !s.fluid_falling)
    }

    /// `getCollisionShape(...).max(Y, x, z)` for cached shapes.
    #[inline]
    pub(crate) fn collision_top_cached<'b>(
        &'b self,
        cached: &mut Option<((i32, i32), Option<&'b crate::chunk::Chunk>)>,
        b: [i32; 3],
        fx: f64,
        fz: f64,
    ) -> f64 {
        let state = self.cached_block(cached, b[0], b[1], b[2]);
        match self.collision.cached_shapes().cached_shape(state) {
            Ok(Some(shape)) if !shape.is_empty() => shape.max_at(Axis::Y, fx, fz),
            Ok(Some(_)) => 0.0,
            _ => {
                if self.facts.get(state).full_collision {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

impl super::Particle {
    pub(super) fn move_by_cached<'b>(
        &mut self,
        level: &'b ParticleLevel,
        cached: &mut Option<((i32, i32), Option<&'b crate::chunk::Chunk>)>,
        d: [f64; 3],
    ) {
        if self.free_move {
            self.bb = self.bb.move_by(Vec3::new(d[0], d[1], d[2]));
            self.set_location_from_bb();
            return;
        }
        if self.stopped_by_collision {
            return;
        }
        let original = d;
        let mut moved = d;
        let length2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
        if self.has_physics && length2 != 0.0 && length2 < 100.0 * 100.0 {
            moved = level.collide_cached(cached, self.bb, d);
        }
        if moved != [0.0; 3] {
            self.bb = self.bb.move_by(Vec3::new(moved[0], moved[1], moved[2]));
            self.set_location_from_bb();
        }
        if original[1].abs() >= f64::from(1.0e-5f32) && moved[1].abs() < f64::from(1.0e-5f32) {
            self.stopped_by_collision = true;
        }
        self.on_ground = original[1] != moved[1] && original[1] < 0.0;
        if original[0] != moved[0] {
            self.velocity[0] = 0.0;
        }
        if original[2] != moved[2] {
            self.velocity[2] = 0.0;
        }
    }

    pub(super) fn base_tick_cached<'b>(
        &mut self,
        level: &'b ParticleLevel,
        cached: &mut Option<((i32, i32), Option<&'b crate::chunk::Chunk>)>,
    ) {
        self.previous = self.position;
        let age = self.age;
        self.age += 1;
        if age >= self.lifetime {
            self.removed = true;
            return;
        }
        self.velocity[1] -= 0.04 * f64::from(self.gravity);
        self.move_by_cached(level, cached, self.velocity);
        if self.speed_up_when_y_blocked && self.position[1] == self.previous[1] {
            self.velocity[0] *= 1.1;
            self.velocity[2] *= 1.1;
        }
        let f = f64::from(self.friction);
        for c in &mut self.velocity {
            *c *= f;
        }
        if self.on_ground {
            self.velocity[0] *= f64::from(0.7f32);
            self.velocity[2] *= f64::from(0.7f32);
        }
    }
}
