//! Mouse Tweaks-style controls (`mouseTweaks`, `mouseTweaksWheel`; #332, `docs/extras.md`). Each
//! action is made of ordinary slot clicks ([`Container::click`]), so it works wherever vanilla's
//! clicks do. They run in the player inventory, the chest-like containers (chests, barrels, ender
//! chests, shulker boxes, hoppers, dispensers), furnaces and crafting tables.
//!
//! - Right-drag holding a stack: one item into each slot entered, at once.
//! - Left-drag holding a stack, before vanilla's drag has spread over a second slot: each slot
//!   entered holding the same item is gathered into the hand (when it all fits); with Shift, those
//!   slots are quick-moved.
//! - Shift + left-drag with an empty hand: every slot entered is quick-moved.
//! - The wheel over a slot, with an empty hand: down moves one of its items where a Shift click
//!   would send it (`quickMoveStack`: fuel to the fuel slot, a smeltable to the input), up brings
//!   one of the same item from the other side. Down over a result slot takes one craft or the whole
//!   furnace output to the inventory; nothing ever goes into a result slot.
use horizon_platform::containers::{Click, Container, Role, SlotId, Stack};

/// A menu the tweaks run in: their clicks are vanilla's in these, and the menu engine knows which
/// slot takes what. Not the menus whose slots cost or trigger something (anvils, enchanting,
/// merchants, beacons, crafters), nor a mount's.
pub fn tweak_menu(name: &str) -> bool {
    name == "inventory"
        || name.starts_with("minecraft:generic_9x")
        || matches!(
            name,
            "minecraft:shulker_box"
                | "minecraft:hopper"
                | "minecraft:generic_3x3"
                | "minecraft:furnace"
                | "minecraft:blast_furnace"
                | "minecraft:smoker"
                | "minecraft:crafting"
        )
}

/// One player's tweaks: the drag they follow, and the wheel's steps not yet acted on (a trackpad
/// scrolls in fractions).
#[derive(Default)]
pub struct Tweaks {
    drag: Option<Drag>,
    wheel: f32,
}

/// A drag the tweaks follow.
#[derive(Clone, Debug)]
struct Drag {
    right: bool,
    /// The stack in hand at the press and its count since, as the drag's own clicks change it (a
    /// server answers them later).
    held: Option<Stack>,
    /// The drag replaced vanilla's: its release clicks nothing.
    active: bool,
    visited: Vec<SlotId>,
}

fn role(c: &dyn Container, slot: SlotId) -> Option<Role> {
    c.slots().into_iter().find(|s| s.id == slot).map(|s| s.role)
}

fn is_result(c: &dyn Container, slot: SlotId) -> bool {
    role(c, slot) == Some(Role::Result)
}

/// The crafting result of the inventory or a crafting table: the drags pass over it.
fn crafting_result(c: &dyn Container, slot: SlotId) -> bool {
    matches!(c.menu(), "inventory" | "minecraft:crafting") && is_result(c, slot)
}

impl Tweaks {
    /// A press on `slot`: true when the tweaks took it (a right-drag placing), else vanilla's press
    /// runs and the tweaks watch the drag.
    pub fn press(
        &mut self,
        on: bool,
        c: &mut dyn Container,
        slot: Option<SlotId>,
        right: bool,
    ) -> bool {
        self.drag = None;
        let Some(slot) = slot.filter(|&s| !crafting_result(c, s)) else {
            return false;
        };
        if !on || !tweak_menu(c.menu()) {
            return false;
        }
        let held = c.carried();
        let shift = c.shift();
        if right {
            // A result slot takes nothing: its press stays vanilla's.
            if shift || held.is_none() || is_result(c, slot) {
                return false;
            }
            // RMB: the press places at once, then the drag goes on.
            let mut drag = Drag {
                right,
                held,
                active: true,
                visited: vec![slot],
            };
            place(c, &mut drag, slot);
            self.drag = Some(drag);
            return true;
        }
        if held.is_some() || shift {
            self.drag = Some(Drag {
                right,
                held,
                active: false,
                visited: vec![slot],
            });
        }
        false
    }

    /// The cursor moved while a button is held: true when the tweaks own the drag (vanilla's quick
    /// craft drag doesn't grow).
    pub fn drag(&mut self, on: bool, c: &mut dyn Container, slot: Option<SlotId>) -> bool {
        if !on || !tweak_menu(c.menu()) {
            return false;
        }
        let Some(mut drag) = self.drag.take() else {
            return false;
        };
        let active = drag.active;
        let entered = slot.filter(|&s| !crafting_result(c, s) && !drag.visited.contains(&s));
        if let Some(slot) = entered {
            drag.visited.push(slot);
            let stack = c.stack(slot);
            let shift = c.shift();
            if drag.right {
                place(c, &mut drag, slot);
            } else if let Some(held) = drag.held.clone() {
                // Holding a stack: only slots of the same item, and only while vanilla's drag
                // hasn't spread.
                let spread = c.spread() > 1;
                if let Some(s) = stack.filter(|s| s.kind == held.kind)
                    && (drag.active || !spread)
                {
                    drag.active = true;
                    if shift {
                        c.click(slot, Click::QuickMove);
                    } else if held.count + s.count <= s.max
                        && c.click(slot, Click::Pick { right: false })
                        && c.click(slot, Click::Pick { right: false })
                    {
                        // Put the hand's stack in the slot, then take it all.
                        drag.held = Some(Stack {
                            count: held.count + s.count,
                            ..held
                        });
                    }
                }
            } else if shift && stack.is_some() {
                drag.active = true;
                c.click(slot, Click::QuickMove);
            }
        }
        let owned = drag.active || active;
        self.drag = Some(drag);
        owned
    }

    /// The button went up: the drag ends.
    pub fn release(&mut self) {
        self.drag = None;
    }

    /// The wheel over the screen: true when the tweaks used it.
    pub fn wheel(
        &mut self,
        on: bool,
        c: &mut dyn Container,
        slot: Option<SlotId>,
        delta: f32,
    ) -> bool {
        if !on || !tweak_menu(c.menu()) || c.carried().is_some() {
            return false;
        }
        let Some(slot) = slot else {
            self.wheel = 0.0;
            return false;
        };
        self.wheel += delta;
        let steps = self.wheel.trunc();
        self.wheel -= steps;
        for _ in 0..steps.abs() as u32 {
            if steps < 0.0 {
                push(c, slot);
            } else {
                pull(c, slot);
            }
        }
        true
    }
}

/// Places one held item in `slot` when it takes it.
fn place(c: &mut dyn Container, drag: &mut Drag, slot: SlotId) {
    let Some(held) = drag.held.clone() else {
        return;
    };
    if c.takes_one(slot, &held) && c.click(slot, Click::Put { one: true }) {
        drag.held = (held.count > 1).then(|| Stack {
            count: held.count - 1,
            ..held
        });
    }
}

/// The slots across from `slot`: a container's or crafting table's face the inventory (hotbar
/// first, from the right), the inventory's face them; without one, hotbar and main inventory
/// trade.
fn other_side(c: &dyn Container, slot: SlotId) -> Vec<SlotId> {
    let slots = c.slots();
    let of = |roles: &[Role]| -> Vec<SlotId> {
        slots
            .iter()
            .filter(|s| roles.contains(&s.role))
            .map(|s| s.id)
            .collect()
    };
    // The inventory's own 2x2 grid trades with nothing but the player's slots.
    let own = if c.menu() == "inventory" {
        Vec::new()
    } else {
        of(&[Role::Storage, Role::Input, Role::Result])
    };
    let player = || {
        let hotbar = of(&[Role::Hotbar]).into_iter().rev();
        hotbar
            .chain(of(&[Role::Inventory]).into_iter().rev())
            .collect()
    };
    match role(c, slot) {
        Some(Role::Storage | Role::Input | Role::Result) => player(),
        Some(Role::Hotbar | Role::Inventory) if !own.is_empty() => own,
        Some(Role::Hotbar) => of(&[Role::Inventory]),
        Some(Role::Inventory) => of(&[Role::Hotbar]),
        _ => Vec::new(),
    }
}

/// Wheel down: one item of `slot` to where a quick move sends it. A result can't be split or put
/// back: one craft (or the furnace's whole output) goes, when it all fits.
fn push(c: &mut dyn Container, slot: SlotId) {
    let Some(stack) = c.stack(slot) else {
        return;
    };
    let landing = c.landing(slot);
    let Some(&target) = landing.first() else {
        return;
    };
    let pick = Click::Pick { right: false };
    if is_result(c, slot) {
        if c.click(slot, pick) {
            for target in landing {
                c.click(target, Click::Put { one: false });
            }
        }
        return;
    }
    if c.click(slot, pick) {
        c.click(target, Click::Put { one: true });
        if stack.count > 1 {
            c.click(slot, Click::Put { one: false });
        }
    }
}

/// Wheel up: one item of the same kind from the other side into `slot`. A result slot takes none
/// and gives none.
fn pull(c: &mut dyn Container, slot: SlotId) {
    let Some(stack) = c.stack(slot) else {
        return;
    };
    if !c.takes_one(slot, &stack) {
        return;
    }
    let mut source = None;
    for s in other_side(c, slot) {
        if let Some(v) = c.stack(s)
            && v.kind == stack.kind
            && !is_result(c, s)
        {
            source = Some((s, v.count));
            break;
        }
    }
    let Some((source, count)) = source else {
        return;
    };
    if c.click(source, Click::Pick { right: false }) {
        c.click(slot, Click::Put { one: true });
        if count > 1 {
            c.click(source, Click::Put { one: false });
        }
    }
}
