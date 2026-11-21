//! The `player_air_right_click` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A guest's `UseItem` packet reached the server: the client sends it for a right click on air,
    /// with no block and no entity under the crosshair. It is heard for every such click, whether the
    /// held item is empty (`item` is `minecraft:air`) or not, and before the server uses the item.
    /// `hand` is `mainhand` or `offhand`; `player` and `name` identify the guest, whose state is swapped in.
    /// A notice only: it cannot cancel the click. Mods that answer a held item's use still do it with
    /// `use_item` or `right_click_item`. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge's PlayerInteractEvent.RightClickEmpty is client-only; Bukkit's PlayerInteractEvent RIGHT_CLICK_AIR)"
    )]
    fn player_air_right_click(player: u128, name: &str, hand: &str, item: &str);
}
