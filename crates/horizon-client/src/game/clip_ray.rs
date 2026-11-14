//! The block ray of `Game::clip_ray` (`BlockGetter.clip`), with or without fluids.
use super::*;

impl Game<'_> {
    /// `clip_ray`; with `fluids`, the first fluid block a hit too, at its near face (`ClipContext.Fluid.ANY`).
    pub(super) fn clip_ray_fluids(
        &self,
        origin: [f64; 3],
        dir: [f64; 3],
        reach: f64,
        fluids: bool,
    ) -> Option<Hit> {
        let mut pos = origin.map(|v| v.floor() as i32);
        let step = dir.map(|d| if d > 0.0 { 1 } else { -1 });
        let delta = dir.map(|d| {
            if d == 0.0 {
                f64::INFINITY
            } else {
                (1.0 / d).abs()
            }
        });
        let mut t_max = [0.0; 3];
        for i in 0..3 {
            let boundary = if dir[i] > 0.0 {
                f64::from(pos[i]) + 1.0
            } else {
                f64::from(pos[i])
            };
            t_max[i] = if dir[i] == 0.0 {
                f64::INFINITY
            } else {
                (boundary - origin[i]) / dir[i]
            };
        }
        let mut face = [0; 3];
        let mut t = 0.0;
        while t <= reach {
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let f = self.res.facts().get(state);
            let has_outline = self.collision.cached_shapes().has_context_shapes(state)
                || (!self.res.facts().is_air(state)
                    && (f.fluid == Fluid::None
                        || f.waterlogged
                        || &*f.name == "minecraft:kelp"
                        || f.render_shape == horizon_client::blocks::RenderShape::Model)
                    && !matches!(
                        &*f.name,
                        "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column"
                    ));
            if has_outline {
                let hit = self.clip_declared_outline(state, pos, origin, dir, reach);
                if hit.is_some() {
                    return hit;
                }
            }
            if fluids
                && f.fluid != Fluid::None
                && !self.res.facts().is_air(state)
                && !self.collision.cached_shapes().has_context_shapes(state)
            {
                return Some(Hit {
                    pos,
                    face,
                    point: [
                        origin[0] + dir[0] * t,
                        origin[1] + dir[1] * t,
                        origin[2] + dir[2] * t,
                    ],
                    // A fluid hit's inside flag is never read: a placement
                    // onto a fluid aims at the block above it.
                    inside: false,
                });
            }
            let axis = if t_max[0] < t_max[1] {
                if t_max[0] < t_max[2] { 0 } else { 2 }
            } else if t_max[1] < t_max[2] {
                1
            } else {
                2
            };
            t = t_max[axis];
            t_max[axis] += delta[axis];
            pos[axis] += step[axis];
            face = [0; 3];
            face[axis] = -step[axis];
        }
        None
    }
}
