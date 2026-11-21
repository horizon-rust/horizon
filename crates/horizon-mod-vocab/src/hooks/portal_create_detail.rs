//! The `portal_create_detail` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0401

pub trait Hooks {
    /// `portal_create` with the blocks: `created` lists the Nether portal blocks of a travel, from
    /// `pos` along the travel axis and then up, row by row; it does not list the obsidian frame,
    /// and is empty for an End platform. Heard right after `portal_create`, from the same place.
    /// A separate hook, because `portal_create` keeps its published signature (#19020).
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn portal_create_detail();
}
