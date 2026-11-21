//! The `ender_pearl_teleport_detail` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0356

pub trait Hooks {
    /// A server player's or mob's ender pearl is about to teleport its `owner`
    /// (`EntityTeleportEvent.EnderPearl`, `ThrownEnderpearl.onHit`), heard after
    /// `ender_pearl_teleport` from the same place, with the write that hook leaves out. The
    /// other fields are that hook's, as it left them, `cancel` included: a pearl it canceled is
    /// heard here only by a listener that receives cancelled events.
    /// `relative` is the position packet's flag set (`Relative`, vanilla's bits), 0 for an
    /// absolute move. The `DELTA_X`, `DELTA_Y` and `DELTA_Z` bits (0x20, 0x40, 0x80) keep that
    /// component of a player owner's motion across the teleport; the others zero it, as vanilla's
    /// absolute move does. The `X`, `Y` and `Z` bits (0x1, 0x2, 0x4) are not read, since `target`
    /// is already absolute, and the rotation bits (0x8, 0x10, 0x100) change nothing: the owner
    /// keeps its look. Mob owners ignore `relative`. Bits outside 0x1ff are ignored.
    /// A separate hook, because `ender_pearl_teleport` keeps its published signature (#19020).
    /// Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "EntityTeleportEvent.EnderPearl",
        cancellable = true
    )]
    fn ender_pearl_teleport_detail(
        owner: EntityRef,
        pearl: u32,
        from: [f64; 3],
        target: &mut [f64; 3],
        relative: &mut u32,
        cancel: &mut bool,
        hit_entity: Option<u32>,
        hit_block: [i32; 3],
        hit_face: [i32; 3],
        hit_at: [f64; 3],
    );
}
