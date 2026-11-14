//! Horizon Extras' sound physics (`horizon_client::sound_physics`): once a tick, the
//! settings, the block states (once; the sound physics thread builds their materials) and
//! the chunk sections around the listener go to the audio side, which traces sounds on a
//! thread of its own. Nothing happens while the extra is off.
use super::Game;
use horizon_client::acoustics::BlockState;
use horizon_client::blocks::Fluid;
use horizon_client::sound_physics::{FluidKind, closes_axes};
use horizon_world::registry::BlockStateId;

impl Game<'_> {
    pub(super) fn tick_sound_physics(&mut self) {
        self.audio
            .set_sound_physics(&self.settings.extras.sound_physics);
        let flying: Vec<(u32, [f64; 3], f32)> = if self.settings.extras.sound_physics.arrow_whoosh {
            self.creatures
                .projectiles
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    !p.stuck
                        && matches!(
                            p.kind,
                            horizon_client::projectiles::ProjectileKind::Arrow { .. }
                        )
                })
                .map(|(i, p)| {
                    // An untracked projectile has id 0: keyed by its slot then.
                    let key = if p.id != 0 { p.id } else { u32::MAX - i as u32 };
                    let v = p.velocity;
                    let speed = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt() * 20.0;
                    (
                        key,
                        [p.position.x, p.position.y, p.position.z],
                        speed as f32,
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        self.audio.set_whooshes(&flying);
        let drawing = self
            .settings
            .extras
            .sound_physics
            .bow_creak
            .then(|| self.bow_hand())
            .flatten();
        self.audio.set_bow_draw(drawing);
        if !self.audio.sound_physics_enabled() {
            return;
        }
        let count = self.res.facts().states.len();
        if self.audio.needs_sound_states(count) {
            let states = self.sound_states();
            self.audio.set_sound_states(states);
        }
        let underwater = self.ambience.was_underwater();
        let chunks = &self.world.chunks;
        self.audio.sync_sound_physics(underwater, |x, z| {
            chunks.get(&(x, z)).map(|c| &c.sections[..])
        });
    }
    /// Where the bow being drawn is: right of the eyes, a little below and
    /// ahead (the view's; a VR hand would place it better). None when the
    /// item in use is not a bow.
    fn bow_hand(&self) -> Option<[f64; 3]> {
        let (slot, _) = self.using?;
        if &*self.stack_in(slot)?.item != "minecraft:bow" {
            return None;
        }
        let p = self.player.as_ref()?.position;
        let yaw = f64::from(self.yaw).to_radians();
        let (forward, right) = ([-yaw.sin(), yaw.cos()], [-yaw.cos(), -yaw.sin()]);
        Some([
            p.x + 0.35 * right[0] + 0.3 * forward[0],
            p.y + 1.62 - 0.3,
            p.z + 0.35 * right[1] + 0.3 * forward[1],
        ])
    }
    /// What the sound physics thread needs of every block state.
    fn sound_states(&self) -> Vec<BlockState> {
        self.res
            .facts()
            .states
            .iter()
            .enumerate()
            .map(|(id, f)| BlockState {
                block: f.name.clone(),
                break_sound: f.sounds.break_sound.clone(),
                step_sound: f.sounds.step.clone(),
                fluid: match f.fluid {
                    Fluid::None => FluidKind::None,
                    Fluid::Water => FluidKind::Water,
                    Fluid::Lava => FluidKind::Lava,
                },
                collides: self
                    .collision
                    .collision_block(BlockStateId(id as u32))
                    .is_ok_and(|b| !b.empty_shape),
                sturdy: f.sturdy_full,
                closes: closes_axes(&f.outline),
            })
            .collect()
    }
}
