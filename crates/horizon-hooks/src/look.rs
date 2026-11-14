//! How the engine draws a few things a crate above may change (docs/layers.md, plan M5): PvP &
//! HUD's look today. Every field's default is vanilla's, so with nothing set the game looks and
//! sounds as vanilla does.
//!
//! Plain data, pushed: a crate changes its own fields of the look of the player whose turn it is
//! ([`update`]; splitscreen players each have theirs: [`crate::seat`]) when its settings load or
//! change, and the engine reads it where it draws ([`engine::get`]). Several crates share the
//! look, each keeping to its fields. Nothing is asked of a crate per frame.
use std::sync::Mutex;

/// What the engine reads. Built from `Look::default()`, then its fields set.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    /// 1.8.9's pictures of a fight: the sword block, the flat item pose and the hand swing.
    pub legacy_pose: bool,
    /// No attack indicator under the crosshair or beside the hotbar.
    pub hide_attack_indicator: bool,
    /// The `entity.player.attack.*` sounds a server sends are left out (1.9 added them with the
    /// attack cooldown; 1.8.9 played none on a hit).
    pub mute_attack_sounds: bool,
    /// The sweep and damage-heart particles a server sends are left out (1.9 added them).
    pub hide_attack_particles: bool,
    /// The block outline, hurt overlay and glint colours the renderer draws with.
    pub frame: Frame,
    /// The first-person shield's size, in percent.
    pub shield_size: u8,
    /// The totem animation's scale.
    pub totem_scale: f32,
    /// How far down the first-person fire overlay moves, as a fraction of its height.
    pub fire_lower: f32,
    /// Lava's fog as with Fire Resistance, without the effect.
    pub clear_lava: bool,
    /// Water's fog as after 30 seconds under water, from the first moment.
    pub clear_water: bool,
    pub chat: Chat,
    /// The status effect icons at the top right are left out (a crate shows the effects itself).
    pub hide_effect_icons: bool,
    /// The time of day this player sees, as `/time set` takes it (0 to 23999); `None` for the
    /// world's. Only what is drawn follows it, never what plays by the world's time.
    pub time_of_day: Option<i64>,
    /// The rain and thunder levels this player sees (0 to 1); `None` for the world's.
    pub weather: Option<(f32, f32)>,
}
impl Default for Look {
    fn default() -> Self {
        Self {
            legacy_pose: false,
            hide_attack_indicator: false,
            mute_attack_sounds: false,
            hide_attack_particles: false,
            frame: Frame::default(),
            shield_size: 100,
            totem_scale: 1.0,
            fire_lower: 0.0,
            clear_lava: false,
            clear_water: false,
            chat: Chat::default(),
            hide_effect_icons: false,
            time_of_day: None,
            weather: None,
        }
    }
}
impl Look {
    /// Whether the sound `name` a server sent is left out ([`Look::mute_attack_sounds`]).
    pub fn mutes_sound(&self, name: &str) -> bool {
        let name = name.strip_prefix("minecraft:").unwrap_or(name);
        self.mute_attack_sounds && name.starts_with("entity.player.attack.")
    }
    /// The day time drawn while the world's is `world`: the same day (the moon's phase follows the
    /// world's), at [`Look::time_of_day`].
    pub fn day_time(&self, world: i64) -> i64 {
        match self.time_of_day {
            Some(t) => world.div_euclid(24000) * 24000 + t,
            None => world,
        }
    }
}

/// What the renderer reads each frame.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frame {
    /// The block outline's colour (`0xAARRGGBB`) and its stroke's scale; `None` for vanilla's.
    pub outline: Option<(u32, f32)>,
    /// The hurt overlay's texel, 0 for vanilla's.
    pub hurt_texel: u32,
    /// The enchantment glint's texel (RGB bytes then 0xFF, `unpack4x8unorm`'s order), 0 for
    /// vanilla's.
    pub glint: u32,
}

/// The chat's extras. Each is off in vanilla.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Chat {
    /// The local time before each message.
    pub timestamps: bool,
    /// A message repeated in a row shows once with its count.
    pub compact: bool,
    /// A click on a message (where it has no click action) copies it.
    pub copy: bool,
    /// Ctrl+F in the chat searches its messages.
    pub search: bool,
}

/// Each seat's look; a seat that never set one has vanilla's.
static LOOKS: Mutex<Vec<Look>> = Mutex::new(Vec::new());

/// Changes the look of the player whose turn it is: `change` sets the fields its crate owns and
/// leaves the others' be.
pub fn update(change: impl FnOnce(&mut Look)) {
    let mut looks = LOOKS.lock().unwrap_or_else(|e| e.into_inner());
    let seat = crate::seat();
    if looks.len() <= seat {
        looks.resize(seat + 1, Look::default());
    }
    change(&mut looks[seat]);
}

/// The engine's side. Only the engine calls it.
pub mod engine {
    use super::*;

    /// The look of the player whose turn it is: vanilla's until something set one.
    pub fn get() -> Look {
        let looks = LOOKS.lock().unwrap_or_else(|e| e.into_inner());
        looks.get(crate::seat()).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_seat_sees_its_own_look_and_vanillas_until_set() {
        // Seats no other test plays, as the looks are shared.
        crate::engine::set_seat(5);
        assert_eq!(engine::get(), Look::default());
        update(|look| look.legacy_pose = true);
        // Another crate's change keeps the first one's.
        update(|look| look.clear_water = true);
        let look = engine::get();
        assert!(look.legacy_pose && look.clear_water);
        crate::engine::set_seat(4);
        assert_eq!(engine::get(), Look::default());
        crate::engine::set_seat(0);
    }

    #[test]
    fn attack_sounds_go_only_when_muted() {
        let mut look = Look::default();
        let attacks = [
            "minecraft:entity.player.attack.strong",
            "entity.player.attack.sweep",
            "minecraft:entity.player.attack.nodamage",
        ];
        assert!(!attacks.iter().any(|name| look.mutes_sound(name)));
        look.mute_attack_sounds = true;
        assert!(attacks.iter().all(|name| look.mutes_sound(name)));
        // 1.8.9 had the hurt sound: it stays.
        assert!(!look.mutes_sound("minecraft:entity.player.hurt"));
    }

    #[test]
    fn a_fixed_time_keeps_the_worlds_day() {
        // Day 9 at 5000: the moon's phase (the day number) stays, the time of day is the chosen.
        let world = 9 * 24000 + 5000;
        let mut look = Look::default();
        assert_eq!(look.day_time(world), world);
        look.time_of_day = Some(18000);
        assert_eq!(look.day_time(world), 9 * 24000 + 18000);
        look.time_of_day = Some(6000);
        assert_eq!(look.day_time(-100), -24000 + 6000);
    }
}
