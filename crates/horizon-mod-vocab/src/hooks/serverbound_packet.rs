//! The `serverbound_packet` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A play packet a player's client sent is about to be handled by the server
    /// (`ServerGamePacketListenerImpl`, one handler per packet type): every packet
    /// the hub reads from that player's input queue, in the order the client sent
    /// them, movement, digging, placing, interaction, chat, the end of the client
    /// tick and pongs included. `player` is the sender's UUID and `packet` the
    /// packet's protocol type id (`minecraft:move_player_pos`,
    /// `minecraft:client_tick_end`, `minecraft:custom_payload`); a custom payload
    /// also exposes its channel and raw bytes, as `client_packet` does. No other
    /// decoded field crosses the API. Return `Flow::Cancel` and the packet is
    /// dropped: no handler runs, nothing is applied and nothing is sent back, as
    /// if the client had never sent it. What vanilla expects of the client stays
    /// the mod's to keep (a teleport confirmation it drops is never confirmed).
    /// There is no rewrite: a mod that wants a different packet cancels this one
    /// and acts through the host. Not heard: custom payloads on a channel a mod
    /// registered (they go to their handler), packets the engine answers before
    /// the player's turn (the configuration phase, login), and the
    /// acknowledgements read while a player is frozen in a handover. Server-side,
    /// for every player of the world (`host.players().acting_player()` says who).
    #[notes = r#"## Serverbound packets

`Hooks::serverbound_packet` receives `&mut dyn ServerHost` and a `ServerboundPacketEvent` with the sender's
`player` (UUID) and a read-only `packet`, the same `ClientPacket` view `client_packet_send` gives a client
mod: the protocol type id, and for a custom payload its channel and bytes. Return `Flow::Cancel` to drop the
packet before the server handles it. This is the point ProtocolLib and PacketEvents give a Paper plugin
(an anticheat reading movement, a rate limiter counting packets); the decoded fields of a packet are not
exposed, so a mod reads state through the host (`player.position(host)`) and uses the packet id for what it
counts or filters.

A packet is held at the head of the player's input queue while the mods rule on it, in the same tick, and
the player's later packets wait behind it, so every mod hears them in the order the client sent them. With
no listener nothing is held, copied or allocated. With one, each packet of each player costs one view and
one pause of the player's turn, so a mod that only counts one packet type pays for all of them: listen only
when you need to."#]
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "none (NeoForge has no raw play-packet receive event)",
        paper = "none (Paper has no public packet API; ProtocolLib and PacketEvents are plugins)",
        cancellable = false
    )]
    fn serverbound_packet();
}
