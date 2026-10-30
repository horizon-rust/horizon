//! The notes capability `rich-events` carries between the game and the
//! runtime (docs/mods.md, "Rich events"). Types only: no state lives here,
//! and nothing of this capability is saved with the world — a mod's own
//! reaction to an event is its business (`storage`, `saved-data`).

/// A mob or player a blow came from, as `hurt-player` names it: the
/// entity's id and kind, and its health before the blow.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityWho {
    pub id: u32,
    /// `minecraft:zombie`, ...; `minecraft:player` for a PvP blow.
    pub kind: String,
    pub health: f32,
}

/// What hurt a player, as the `hurt-player` ask tells the mods.
#[derive(Clone, Debug, PartialEq)]
pub struct DamageSource {
    /// The damage type id, `minecraft:mob_attack`, `minecraft:fall`.
    pub kind: String,
    /// The mob or player that caused it, when there is one.
    pub cause: Option<EntityWho>,
}

/// One of the mobs the server spawned, for the `mobs-spawned` notice.
#[derive(Clone, Debug, PartialEq)]
pub struct MobSpawnNote {
    /// `minecraft:zombie`, ...
    pub kind: String,
    pub dimension: String,
    pub pos: [f64; 3],
    /// Natural spawning made it (the dark, a random spawn); false: a
    /// spawner block, a raid or patrol, a jockey, `/summon`.
    pub natural: bool,
}

/// Pickup asks one player is asked a tick; past them vanilla takes the rest.
pub const RICH_PICKUP_ASKS: usize = 16;
/// Spawns one `mobs-spawned` call is told of; the rest spawn unsaid and are
/// logged. The game keeps its note list at this size.
pub const RICH_SPAWNS_PER_TICK: usize = 64;
