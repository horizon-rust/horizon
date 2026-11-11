//! The extension point for resource pack formats' own particles (Polytone's custom particle types,
//! block and entity emitters, particle modifiers and GPU particles, #11486), which the platform
//! fills ([`crate::pack_hooks::PackFormats::custom_particles`]). A custom particle is an ordinary
//! engine particle with a controller, or a GPU particle: a record its store draws.
use super::{Particle, ParticleEngine, ParticleLevel, ParticleLightCache, ParticleOptions};
use crate::environment::Frame;
use crate::hud::Rect;
use crate::mesher::Vertex;
use glam::{DVec3, Mat3, Quat, Vec3 as FVec3};
use horizon_core::random::LegacyRandom;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// What an emitter seeds on the custom particle it spawns: its own `red`... `custom` and customs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Seed {
    pub fields: BTreeMap<String, f64>,
    pub customs: BTreeMap<String, f64>,
}

/// A particle to spawn: its options, position and velocity.
pub type Spawn = (ParticleOptions, [f64; 3], [f64; 3]);

/// A sound for the engine's queue: id, position, volume, pitch, delayed by the distance.
pub type Sound = (&'static str, [f64; 3], f32, f32, bool);

/// A custom particle as it is made.
pub enum Made {
    /// An engine particle, with the controller that runs its type.
    Particle(Particle),
    /// A GPU particle's spawn: only a record in its type's store, never an engine particle.
    Gpu(Particle),
}

/// The packs' custom particle types and emitters, read once per pack load.
pub trait CustomParticles: Send + Sync {
    fn is_empty(&self) -> bool;
    /// Whether `id` (`minecraft:rain`, `ns:spark`) is a custom particle type.
    fn has_type(&self, id: &str) -> bool;
    /// Whether any pack modifies vanilla or mod particles.
    fn modifies_particles(&self) -> bool;
    /// Runs the modifiers aimed at particle type `id` on a particle that was just made; `source`
    /// is the block or item its options carry.
    fn modify(&self, id: &str, source: Option<&str>, particle: &mut Particle);
    /// Makes a particle of custom type `name`; `None` when its type's limits refuse it.
    fn make(
        &self,
        engine: &ParticleEngine,
        name: &str,
        options: &ParticleOptions,
        position: [f64; 3],
        velocity: [f64; 3],
        seed: i64,
    ) -> Option<Made>;
    /// The store for GPU particle type `name`'s records.
    fn gpu_store(&self, name: &str) -> Option<Box<dyn GpuParticles>>;
    /// The sounds custom particles played since the last call.
    fn take_sounds(&self) -> Vec<Sound>;
    /// Whether the packs stop `state`'s own particles (its `Block.animateTick` doesn't run).
    fn disables_particles(&self, state: horizon_world::registry::BlockStateId) -> bool;
    /// The particles the emitters aimed at `entity` (its type) standing at `position` spawn.
    fn entity(
        &self,
        level: &ParticleLevel<'_>,
        entity: &str,
        position: [f64; 3],
        camera: [f64; 3],
        random: &mut LegacyRandom,
        out: &mut Vec<Spawn>,
    );
    /// The particles and sounds the emitters aimed at the block at `at` spawn, from its animate
    /// tick.
    fn animate(
        &self,
        level: &ParticleLevel<'_>,
        at: [i32; 3],
        random: &mut LegacyRandom,
        out: &mut Vec<Spawn>,
        sounds: &mut Vec<Sound>,
    );
    /// Sets the world's biome table (the world's own, or the default one).
    fn set_biomes(&self, biomes: &Arc<crate::biome::Biomes>);
    /// Sets the frame's global, player and camera context the expressions read.
    fn set_frame(&self, frame: &Frame);
    /// The frame last set.
    fn frame(&self) -> Option<Frame>;
}

/// A GPU particle type's records, drawn beside the engine's own particles.
pub trait GpuParticles: Send + Sync {
    /// Whether it holds custom type `name`'s records.
    fn is(&self, name: &str) -> bool;
    /// Records a spawn from a particle its type set up, at engine tick `now`.
    fn spawn(&mut self, particle: &Particle, now: u64, seed: u64);
    /// Forgets the records whose life is over.
    fn expire(&mut self, now: u64);
    /// How many records are live.
    fn records(&self) -> usize;
    /// The quads of every live record at `now + partial` ticks.
    #[allow(clippy::too_many_arguments)]
    fn render(
        &self,
        atlas: &HashMap<String, (Rect, bool)>,
        camera: DVec3,
        facing: Quat,
        now: u64,
        partial: f32,
        light_cache: &mut ParticleLightCache,
        light_at: &dyn Fn([i32; 3]) -> u8,
        out: &mut Vec<Vertex>,
        additive: &mut Vec<Vertex>,
    );
}

/// How a particle quad turns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    /// Faces the camera.
    #[default]
    Camera,
    /// Turns around Y only.
    AroundY,
    /// Lies flat, facing up.
    Flat,
    /// Stands along its velocity, facing the camera around it.
    Movement,
    /// Fixed in the world's axes (its +Z facing south).
    Fixed,
}

/// The rotation of a quad (corners in its XY plane, facing +Z) at `center` (relative to the
/// camera) moving along `velocity`, rolled by `roll`; `camera` is the camera-facing rotation.
pub fn orient(mode: Orientation, center: FVec3, velocity: FVec3, roll: f32, camera: Quat) -> Quat {
    let rolled = |q: Quat| q * Quat::from_rotation_z(roll);
    match mode {
        Orientation::Camera => rolled(camera),
        Orientation::AroundY => rolled(Quat::from_rotation_y((-center.x).atan2(-center.z))),
        Orientation::Flat => rolled(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        Orientation::Fixed => rolled(Quat::IDENTITY),
        Orientation::Movement => {
            let up = velocity.normalize_or_zero();
            let right = up.cross((-center).normalize_or_zero()).normalize_or_zero();
            if right == FVec3::ZERO {
                return rolled(camera);
            }
            Quat::from_mat3(&Mat3::from_cols(right, up, right.cross(up)))
        }
    }
}

impl ParticleOptions {
    /// These options with the block or item they carry (`source`), for particle modifier
    /// filters.
    pub fn with_source(mut self, id: &str) -> Self {
        self.source = Some(Arc::from(id));
        self
    }
}

impl Particle {
    /// Draws sprite `index` of the engine's sprite table.
    pub fn set_sprite_index(&mut self, index: u32) {
        self.sprite = super::Sprite::Index(index);
    }

    /// Draws the atlas sprite `name`.
    pub fn set_sprite_named(&mut self, name: String) {
        self.sprite = super::Sprite::Named(name);
    }

    /// Lights it with at least block light `level`, a floor under the light where it is.
    pub fn set_min_block_light(&mut self, level: u8) {
        self.light = super::Light::MinBlock(level);
    }

    /// Ticks it with its controller alone; it draws translucent or cut out.
    pub fn set_controlled(&mut self, translucent: bool) {
        self.behaviour = super::Behaviour::Controlled { translucent };
    }

    /// The controller that runs it.
    pub fn set_controller(&mut self, controller: impl super::ParticleController) {
        self.controller = Some(Box::new(controller));
    }
}

impl ParticleEngine {
    /// The live and queued particles, for a custom type's `limit` and `exclusion_radius`.
    pub fn live(&self) -> impl Iterator<Item = &Particle> {
        self.particles
            .iter()
            .chain(&self.pending)
            .filter(|p| !p.removed)
    }

    /// The sprite set of particle type `name` and its sprites: a native mod's, or for
    /// `minecraft:<type>` that vanilla type's.
    pub fn sprite_set(&self, name: &str) -> Option<(u32, &[u32])> {
        let set = self.mod_sets.get(name).map(|&(set, _)| set).or_else(|| {
            let vanilla = name.strip_prefix("minecraft:")?;
            self.sets
                .get(&super::ParticleType::from_name(vanilla)?)
                .copied()
        })?;
        let sprites = self
            .set_sprites
            .get(set as usize)
            .map_or(&[][..], Vec::as_slice);
        Some((set, sprites))
    }

    /// The emitters of the packs' entity modifiers aimed at `entity` (its type, as
    /// `minecraft:player`) standing at `position`, run once a client tick.
    pub fn polytone_entity(&mut self, level: &ParticleLevel<'_>, entity: &str, position: [f64; 3]) {
        let Some(custom) = self.polytone.clone() else {
            return;
        };
        let mut out = Vec::new();
        custom.entity(
            level,
            entity,
            position,
            self.camera,
            &mut self.random,
            &mut out,
        );
        for (options, position, velocity) in out {
            self.add(&options, position, velocity);
        }
    }

    /// Whether the packs' block modifiers stop `state`'s own particles (`disable_particles`): its
    /// `Block.animateTick` doesn't run.
    pub fn polytone_disables_particles(
        &self,
        state: horizon_world::registry::BlockStateId,
    ) -> bool {
        self.polytone
            .as_ref()
            .is_some_and(|c| c.disables_particles(state))
    }

    /// The block emitters of the packs' block modifiers aimed at the block at `at`, run from a
    /// block animate tick (`Block.animateTick`, as torches spawn their flames).
    pub fn polytone_animate(&mut self, level: &ParticleLevel<'_>, at: [i32; 3]) {
        let Some(custom) = self.polytone.clone() else {
            return;
        };
        let (mut out, mut sounds) = (Vec::new(), Vec::new());
        custom.animate(level, at, &mut self.random, &mut out, &mut sounds);
        for (options, position, velocity) in out {
            self.add(&options, position, velocity);
        }
        self.sounds.extend(sounds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientations_turn_the_quad_normal() {
        let normal = |q: Quat| q * FVec3::Z;
        let close = |a: FVec3, b: FVec3| (a - b).length() < 1e-5;
        // Lying flat: facing up.
        assert!(close(
            normal(orient(
                Orientation::Flat,
                FVec3::ONE,
                FVec3::ZERO,
                0.0,
                Quat::IDENTITY
            )),
            FVec3::Y
        ));
        // Around Y: facing the camera horizontally (the camera at the origin, the quad east).
        assert!(close(
            normal(orient(
                Orientation::AroundY,
                FVec3::new(5.0, 3.0, 0.0),
                FVec3::ZERO,
                0.0,
                Quat::IDENTITY
            )),
            -FVec3::X
        ));
        // Along its motion: its up is the velocity, its face toward the camera.
        let q = orient(
            Orientation::Movement,
            FVec3::new(0.0, 0.0, -4.0),
            FVec3::new(0.0, -2.0, 0.0),
            0.0,
            Quat::IDENTITY,
        );
        assert!(close(q * FVec3::Y, -FVec3::Y));
        assert!(close(normal(q), FVec3::Z));
    }
}
