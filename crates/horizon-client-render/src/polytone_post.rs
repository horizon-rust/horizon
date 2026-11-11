//! Resource packs' post chains beyond vanilla's (#11486; Polytone's, wiki "Shaders"): vanilla
//! `post_effect` chains a pack runs over the world while a condition holds, with uniform blocks
//! of their own, the `PolyGlobals` block, and targets kept between frames. `post_chain` runs
//! them; the platform's pack formats read them ([`PostChains`]) once per pack load.
use crate::environment::Frame;
use crate::post_chain::Extras;
use std::path::Path;

/// A pack format's post chains: which run this frame, and what they add.
pub trait PostChains: Send + Sync {
    /// The chains to run this frame, and their blocks and targets.
    fn frame(&self, frame: &Frame, globals: &PolyGlobals) -> (Vec<String>, Extras);
}

/// The packs' post chains and targets.
pub struct PolytonePost(Box<dyn PostChains>);

impl PolytonePost {
    /// The post chains and targets of the packs merged at `root`, with the packs' option values;
    /// `None` without chains.
    pub fn read(root: &Path, configs: horizon_pack_options::Configs) -> Option<Self> {
        crate::pack_hooks::formats()
            .post_chains(root, configs)
            .map(Self)
    }

    /// The chains to run this frame (those whose condition holds), and what they add.
    pub fn frame(&self, frame: &Frame, globals: &PolyGlobals) -> (Vec<String>, Extras) {
        self.0.frame(frame, globals)
    }
}

/// What `PolyGlobals` holds (wiki "Extra Shader Uniforms").
#[derive(Clone, Copy, Debug, Default)]
pub struct PolyGlobals {
    /// The world's projection (reverse-Z, as 26.3's and Horizon's are), and its model-view (the
    /// camera's turn).
    pub projection: glam::Mat4,
    pub model_view: glam::Mat4,
    /// The sun's angle in radians, 0 at the horizon.
    pub sun_angle: f32,
    /// The time of day in ticks (0 to 24000), and the game ticks this frame advanced.
    pub day_time: f32,
    pub delta_time: f32,
    /// The player's feet: `PolyPlayerBlockPos`, and `PolyPlayerOffset` as vanilla's
    /// `CameraOffset` is (the block position less the position, so `polyPlayerPos()` is the
    /// block position less the offset).
    pub player: [f64; 3],
}

impl PolyGlobals {
    /// The std140 block: `mat4, mat4, float, float, float, ivec3, vec3`.
    pub fn bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(176);
        for m in [self.projection, self.model_view] {
            out.extend(m.to_cols_array().iter().flat_map(|v| v.to_le_bytes()));
        }
        for v in [self.sun_angle, self.day_time, self.delta_time, 0.0] {
            out.extend(v.to_le_bytes());
        }
        let block = self.player.map(f64::floor);
        for v in block {
            out.extend((v as i32).to_le_bytes());
        }
        out.extend(0i32.to_le_bytes());
        for i in 0..3 {
            out.extend(((block[i] - self.player[i]) as f32).to_le_bytes());
        }
        out.extend(0f32.to_le_bytes());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poly_globals_follow_std140() {
        let g = PolyGlobals {
            projection: glam::Mat4::from_diagonal(glam::Vec4::splat(2.0)),
            sun_angle: 1.5,
            day_time: 6000.0,
            delta_time: 0.5,
            player: [10.25, -3.5, 7.0],
            ..Default::default()
        };
        let b = g.bytes();
        let f = |at: usize| f32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        let i = |at: usize| i32::from_le_bytes(b[at..at + 4].try_into().unwrap());
        assert_eq!(b.len(), 176);
        assert_eq!(f(0), 2.0);
        assert_eq!((f(128), f(132), f(136)), (1.5, 6000.0, 0.5));
        // ivec3 at 144, vec3 at 160 (std140 aligns both to 16). The offset is the block position
        // less the position, as vanilla's CameraOffset: the block position less it is the position.
        assert_eq!((i(144), i(148), i(152)), (10, -4, 7));
        assert_eq!((f(160), f(164), f(168)), (-0.25, -0.5, 0.0));
    }
}
