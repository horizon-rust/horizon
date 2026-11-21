//! The `ban_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0352

pub trait Hooks {
    /// An entry was added to (`added`) or removed from one of the dedicated server's ban lists: the list
    /// (`BanListKind`) and the entry as `server_ban_list` reports it. Covers `/ban`, `/pardon`, `/ban-ip`,
    /// `/pardon-ip`, the management protocol, the chat filter and the anticheat. A `clear` says one removal for
    /// each entry. Heard at the start of the next server tick, in the order the lists changed; a change made
    /// before the first tick a mod listens is not heard. At most 4096 changes wait between two ticks, the oldest
    /// are dropped with no notice. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no ban event)",
        paper = "none (Paper fires no event for a ban-list change)"
    )]
    fn ban_changed();
}
