//! PURPOSE: `Entity.updateFluidInteraction` current for vehicles (minecart, boat): flowing
//! water (0.014) and lava (0.007 / 0.0023333) add a push to the entity's motion.
//! WHY: `Entity.isPushedByFluid` is true for every entity that does not override it, and
//! carts and boats did not call it, so a stream left them still.
//! REFS: Entity.updateFluidHeightAndDoFluidPushing; same adapter as tnt.rs (TntFluidWorld).
//! LOGS: none.
//! READS-BY: minecarts.rs `tick_cart`, boats.rs `tick_boat`.
//! ENDS-WHEN: a shared fluid adapter replaces the per-entity ones (tnt.rs, arrows.rs).
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
};
use horizon_world::{
    entity_fluid::{
        CurrentTarget, EntityFluidInteraction, FluidInteractionWorld, FluidMembership,
        FluidSections, FluidTagId,
    },
    fluid::{FluidHeightWorld, FluidKind, FluidState},
    fluid_flow::{FluidFlowWorld, flow_at},
};

const WATER: FluidTagId = FluidTagId(0);
const LAVA: FluidTagId = FluidTagId(1);

/// Update the fluid contact of a body (`bounds` at `position`) and push `velocity` by the
/// water and lava currents, like `Entity.updateFluidInteraction`. Returns whether the body
/// is in water.
pub(super) fn push_by_fluid(
    world: &horizon_client::world::World,
    assets: &horizon_client::assets::Assets,
    facts: &horizon_client::blocks::BlockFacts,
    bounds: Aabb,
    position: Vec3,
    velocity: &mut Vec3,
) -> std::io::Result<bool> {
    let mut fluids = EntityFluidInteraction::new([WATER, LAVA]);
    let mut w = BodyFluidWorld {
        bounds,
        position,
        velocity,
        world,
        assets,
        facts,
    };
    fluids
        .update(&mut w, false, 4096)
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    fluids.apply_current(&mut w, WATER, 0.014)?;
    fluids.apply_current(
        &mut w,
        LAVA,
        if world.fast_lava {
            0.007
        } else {
            0.0023333333333333335
        },
    )?;
    Ok(fluids.is_in_fluid(&w, WATER))
}

struct BodyFluidWorld<'a> {
    bounds: Aabb,
    position: Vec3,
    velocity: &'a mut Vec3,
    world: &'a horizon_client::world::World,
    assets: &'a horizon_client::assets::Assets,
    facts: &'a horizon_client::blocks::BlockFacts,
}
impl BodyFluidWorld<'_> {
    fn state(&self, p: BlockPos) -> horizon_world::registry::BlockStateId {
        self.world.block(p.x, p.y, p.z)
    }
    fn fluid_at(&self, p: BlockPos) -> std::io::Result<FluidState> {
        FluidState::from_block(
            self.assets
                .registry
                .state(self.state(p))
                .ok_or_else(|| std::io::Error::other("missing fluid state"))?,
        )
    }
}
impl FluidMembership for BodyFluidWorld<'_> {
    fn fluid_in_tag(&self, kind: FluidKind, tag: FluidTagId) -> bool {
        match tag {
            WATER => matches!(kind, FluidKind::Water | FluidKind::FlowingWater),
            LAVA => matches!(kind, FluidKind::Lava | FluidKind::FlowingLava),
            _ => false,
        }
    }
}
impl CurrentTarget for BodyFluidWorld<'_> {
    type Error = std::io::Error;
    fn is_player_class(&self) -> bool {
        false
    }
    fn velocity(&mut self) -> std::io::Result<Vec3> {
        Ok(*self.velocity)
    }
    fn add_delta_movement(&mut self, v: Vec3) -> std::io::Result<()> {
        *self.velocity = Vec3::new(
            self.velocity.x + v.x,
            self.velocity.y + v.y,
            self.velocity.z + v.z,
        );
        Ok(())
    }
}
impl FluidInteractionWorld for BodyFluidWorld<'_> {
    type Error = std::io::Error;
    type Chunk = (i32, i32);
    fn interaction_box(&mut self) -> std::io::Result<Option<Aabb>> {
        Ok(Some(self.bounds.deflate(Vec3::new(0.001, 0.001, 0.001))))
    }
    fn loaded_chunk(&mut self, x: i32, z: i32) -> std::io::Result<Option<Self::Chunk>> {
        Ok(self.world.chunks.contains_key(&(x, z)).then_some((x, z)))
    }
    fn chunk_sections(&mut self, _: Self::Chunk) -> std::io::Result<FluidSections<'_>> {
        let layout = self.world.layout();
        Ok(FluidSections {
            minimum_section_y: layout.min_y() / 16,
            has_fluid: horizon_client::chunk::every_section(layout),
        })
    }
    fn bounding_box_min_y(&self) -> f64 {
        self.bounds.min_position().y
    }
    fn block_x(&self) -> i32 {
        self.position.x.floor() as i32
    }
    fn block_z(&self) -> i32 {
        self.position.z.floor() as i32
    }
    fn eye_y(&mut self) -> std::io::Result<f64> {
        Ok(self.bounds.max_position().y)
    }
    fn fluid_state(&mut self, p: BlockPos) -> std::io::Result<FluidState> {
        self.fluid_at(p)
    }
    fn fluid_height(&mut self, s: FluidState, p: BlockPos) -> std::io::Result<f32> {
        s.height_at(self, [p.x, p.y, p.z])
    }
    fn camera_height(&mut self, s: FluidState, p: BlockPos) -> std::io::Result<f32> {
        s.camera_height_at(self, [p.x, p.y, p.z])
    }
    fn flow(&mut self, s: FluidState, p: BlockPos) -> std::io::Result<Vec3> {
        flow_at(self, s, p)
    }
}
impl FluidHeightWorld for BodyFluidWorld<'_> {
    type Error = std::io::Error;
    fn fluid(&mut self, p: [i32; 3]) -> std::io::Result<FluidState> {
        self.fluid_at(BlockPos::new(p[0], p[1], p[2]))
    }
    fn sturdy_down(&mut self, p: [i32; 3]) -> std::io::Result<bool> {
        Ok(self
            .facts
            .get(self.state(BlockPos::new(p[0], p[1], p[2])))
            .sturdy_full
            & 1
            != 0)
    }
}
impl FluidFlowWorld for BodyFluidWorld<'_> {
    type Error = std::io::Error;
    type Block = horizon_world::registry::BlockStateId;
    fn fluid(&mut self, p: BlockPos) -> std::io::Result<FluidState> {
        self.fluid_at(p)
    }
    fn block(&mut self, p: BlockPos) -> std::io::Result<Self::Block> {
        Ok(self.state(p))
    }
    fn blocks_fluid_flow(&self, s: Self::Block) -> bool {
        self.assets
            .blocks_fluid_flow
            .contains(&self.assets.registry.state(s).unwrap().name)
    }
    fn is_ice_class(&self, s: Self::Block) -> bool {
        self.facts
            .get(s)
            .classes
            .iter()
            .any(|c| c.as_ref() == "IceBlock")
    }
    fn is_face_sturdy(
        &mut self,
        s: Self::Block,
        _: BlockPos,
        d: horizon_core::geometry::Direction,
    ) -> std::io::Result<bool> {
        Ok(self.facts.get(s).sturdy_full & (1 << d as u8) != 0)
    }
}
