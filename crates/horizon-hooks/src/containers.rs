//! Container screens (docs/layers.md, "Container screens"): input and drawing on an open container
//! screen (the inventory, chests, furnaces, crafting tables...) for a crate above the engine, as
//! Mouse Tweaks and sorting need. The engine implements [`Container`] for the length of one call
//! of a hook (`Hooks::container_press` and the others); everything in it is plain data.

/// A slot of the open screen: opaque, stable while the screen is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SlotId(pub u32);

/// What a slot is for.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// The player's hotbar.
    Hotbar,
    /// The player's main inventory, above the hotbar.
    Inventory,
    /// A slot that only holds items: a chest's, a hopper's, a mount's chest.
    Storage,
    /// A slot a recipe or a furnace reads: a crafting grid's, a furnace's input and fuel.
    Input,
    /// A slot that only gives: a crafting or furnace result.
    Result,
    /// The player's armour and off hand, a mount's saddle and armour.
    Equipment,
    Other,
}

/// A slot drawn on the screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub id: SlotId,
    pub role: Role,
    /// Its top left corner in GUI pixels, from the panel's.
    pub x: f32,
    pub y: f32,
}

/// The items in a slot or on the cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stack {
    /// The item's id (`minecraft:stone`).
    pub item: String,
    pub count: u32,
    /// How many one slot holds of it.
    pub max: u32,
    /// Equal for stacks of the same item with the same components: those merge.
    pub kind: u64,
}

/// One ordinary slot click, as vanilla's screen sends it.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Click {
    /// Picks up, puts down or swaps (`PICKUP`), the left or right button.
    Pick { right: bool },
    /// The Shift click (`QUICK_MOVE`).
    QuickMove,
    /// Puts the cursor's stack, or one of it, in the slot as a one-slot drag (`QUICK_CRAFT`): a
    /// server whose cursor is already empty ignores it, where a click would pick up the slot.
    Put { one: bool },
}

/// The open screen's menu, for the length of one call. The engine implements it. The reads that
/// ask the menu's rules take `&mut self`, as the engine's do; drawing gets the others only.
pub trait Container {
    /// Its menu type: `inventory` for the player's own, else the registry name the server opened
    /// (`minecraft:generic_9x3`, `minecraft:furnace`...).
    fn menu(&self) -> &str;
    /// Every slot drawn, in the screen's order, in every menu: a crate keeps to the menus it acts
    /// in, by [`Container::menu`] (Mouse Tweaks stays out of those whose slots cost or trigger
    /// something: anvils, enchanting, merchants, beacons, crafters). Creative's tabs are no
    /// container screen.
    fn slots(&self) -> Vec<Slot>;
    fn stack(&mut self, slot: SlotId) -> Option<Stack>;
    /// The stack on the cursor.
    fn carried(&self) -> Option<Stack>;
    /// Whether `slot` takes one more of `of`, by the menu's rules (`mayPlace`, the slot's limit).
    fn takes_one(&mut self, slot: SlotId, of: &Stack) -> bool;
    /// Where a Shift click on `slot` would put its items (`quickMoveStack`), first first.
    fn landing(&mut self, slot: SlotId) -> Vec<SlotId>;
    /// One ordinary click. False when nothing took it.
    fn click(&mut self, slot: SlotId, click: Click) -> bool;
    /// How many slots vanilla's drag of the cursor's stack spreads over now (0 while none runs).
    fn spread(&self) -> usize;
    /// The cursor, in GUI pixels from the panel's corner.
    fn cursor(&self) -> (f32, f32);
    /// Whether Shift is held.
    fn shift(&self) -> bool;
    /// The creative tabs' order of the stack in `slot` (`u32::MAX` when no tab lists it) and the
    /// name it shows: how sorting ranks it.
    fn order(&mut self, slot: SlotId) -> (u32, String);
}
