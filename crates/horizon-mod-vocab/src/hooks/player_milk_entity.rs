//! The `player_milk_entity` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player milked a cow, a mooshroom or a goat with a bucket, after the bucket was
    /// used up and the milk bucket given (in the hand, else the inventory, else dropped).
    /// `entity_id` is the animal's entity id and `entity` its species id
    /// (`minecraft:cow`, `minecraft:mooshroom`, `minecraft:goat`). A bowl on a mooshroom is not
    /// milking and is not heard. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_milk_entity();
}
