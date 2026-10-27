//! Horizon Extras' sound physics (`horizon_client::sound_physics`): once a tick, the
//! settings, the block states (once; the sound physics thread builds their materials) and
//! the chunk sections around the listener go to the audio side, which traces sounds on a
//! thread of its own. Nothing happens while the extra is off.
use super::Game;
use horizon_client::acoustics::BlockState;
use horizon_client::blocks::Fluid;
use horizon_client::sound_physics::FluidKind;
use horizon_world::registry::BlockStateId;

impl Game<'_> {
    pub(super) fn tick_sound_physics(&mut self) {
        self.audio
            .set_sound_physics(&self.settings.extras.sound_physics);
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
            })
            .collect()
    }
}
