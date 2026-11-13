//! Constructors for events produced by a mob tick.
use super::{Mob, MobEvent};

impl MobEvent {
    pub fn entity_sound(
        mob: &Mob,
        name: &'static str,
        source: usize,
        volume: f32,
        pitch: f32,
    ) -> Self {
        Self::EntitySound {
            mob: mob.id,
            name,
            at: [mob.position.x, mob.position.y, mob.position.z],
            volume,
            pitch,
            source,
        }
    }
}
