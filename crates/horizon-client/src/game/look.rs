//! What a crate above may change in how the game draws (`horizon_hooks::look`): the engine reads
//! it here only, and each reading is vanilla's while nothing set a look. PvP & HUD sets it today
//! (`game/extras/pvp.rs`).
use super::{Dim, Game};
use horizon_client::{chat::ChatQol, particles::ParticleType};
use horizon_hooks::look::{Chat, Frame, engine::get as look};

impl Game<'_> {
    /// 1.8.9's pictures of a fight: the sword block, the flat item pose and the hand swing.
    pub(super) fn legacy_pose(&self) -> bool {
        look().legacy_pose
    }
    pub(super) fn attack_indicator_hidden(&self) -> bool {
        look().hide_attack_indicator
    }
    /// A sound a server sent that this player doesn't hear (`Look::mutes_sound`).
    pub(super) fn sound_muted(&self, name: &str) -> bool {
        look().mutes_sound(name)
    }
    /// A particle a server sent that this player doesn't see: with the attack particles hidden,
    /// the sweep and the damage hearts, which 1.9 added to a hit (1.8.9's show crits only).
    pub(super) fn particle_hidden(&self, ty: ParticleType) -> bool {
        let attack = matches!(
            ty,
            ParticleType::SweepAttack | ParticleType::DamageIndicator
        );
        attack && look().hide_attack_particles
    }
    /// The block outline, hurt overlay and glint colours the renderer draws with.
    pub(super) fn look_frame(&self) -> Frame {
        look().frame
    }
    /// The enchantment glint's colour texel, 0 for vanilla's.
    pub(super) fn glint_texel(&self) -> u32 {
        look().frame.glint
    }
    /// The held shield's size in percent (100 is vanilla's).
    pub(super) fn shield_size(&self) -> u8 {
        look().shield_size
    }
    /// The totem animation's scale (1 is vanilla's).
    pub(super) fn totem_scale(&self) -> f32 {
        look().totem_scale
    }
    /// How far the first-person fire overlay is lowered, as a fraction (0 is vanilla's).
    pub(super) fn fire_lower(&self) -> f32 {
        look().fire_lower
    }
    /// Lava's fog as with fire resistance, and water's as fully adapted.
    pub(super) fn clear_fluids(&self) -> (bool, bool) {
        let look = look();
        (look.clear_lava, look.clear_water)
    }
    /// The chat's extras: Ctrl+F search, copy on click, timestamps, repeats counted.
    pub(super) fn chat_look(&self) -> Chat {
        look().chat
    }
    /// What the chat lays its lines out with.
    pub(super) fn chat_qol(&self) -> ChatQol {
        let chat = look().chat;
        ChatQol {
            timestamps: chat.timestamps,
            compact: chat.compact,
        }
    }
    /// Whether vanilla's status effect icons are left out (`Hud.extractEffects`).
    pub(super) fn effect_icons_hidden(&self) -> bool {
        look().hide_effect_icons
    }
    /// Whether this window may draw its own time and weather: only a client of a server (single
    /// player's included) draws them, never the server that ticks the world.
    fn own_view(&self) -> bool {
        self.remote.is_some() && self.dim == Dim::Overworld
    }
    /// The day time the frame draws (sky, sun, moon, the sky's light): the look's time of day,
    /// else the world's. The world's own `day_time` and everything that plays by it stay.
    pub(super) fn view_day_time(&self) -> i64 {
        if !self.own_view() {
            return self.world.day_time;
        }
        look().day_time(self.world.day_time)
    }
    /// The rain and thunder levels the frame draws (rain, the sky's colour and light, fog): the
    /// look's weather, else the world's. Gameplay (riptide, mobs, crops) keeps the world's.
    pub(super) fn view_weather(&self, partial: f32) -> (f32, f32) {
        match look().weather {
            Some(levels) if self.own_view() && self.can_have_weather() => levels,
            _ => self.weather_levels(partial),
        }
    }
}
