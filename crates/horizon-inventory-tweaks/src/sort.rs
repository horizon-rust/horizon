//! Sorting (`sort`, `sortKey`; #332, `docs/extras.md`): a small button by a menu's storage slots
//! and one by the inventory's (an optional key sorts the part under the cursor). The clicks come
//! from [`crate::plan::plan`] and are ordinary left clicks, so they work wherever vanilla's do.
use crate::plan::{self, Rank};
use horizon_platform::{
    containers::{Click, Container, Role, SlotId, Stack},
    hud::Canvas,
};
use std::collections::HashMap;

/// A sort button's side in GUI pixels.
const SIZE: f32 = 9.0;
const EDGE: u32 = 0xFF37_3737;
const FACE: u32 = 0xFFA0_A0A0;
const FACE_HOVERED: u32 = 0xFFDC_DCDC;

/// The sort buttons of the open screen: their corner (from the panel's) and the slots each sorts.
/// Any menu screen gets them, by its slots' roles: one over the top right of its storage slots (a
/// chest's title line) when it has some, one over the main inventory's (the "Inventory" label's
/// line, `inventoryLabelY`, 12 above the first row).
fn buttons(c: &dyn Container) -> Vec<((f32, f32), Vec<SlotId>)> {
    let slots = c.slots();
    let mut buttons = Vec::new();
    for role in [Role::Storage, Role::Inventory] {
        let of: Vec<_> = slots.iter().filter(|s| s.role == role).collect();
        if of.is_empty() {
            continue;
        }
        // Over the section's last column: (160, 5) over a chest's slots.
        let right = of.iter().map(|s| s.x + 8.0).fold(f32::MIN, f32::max);
        let top = of.iter().map(|s| s.y - 13.0).fold(f32::MAX, f32::min);
        buttons.push(((right, top), of.iter().map(|s| s.id).collect()));
    }
    buttons
}

fn hit(c: &dyn Container, (x, y): (f32, f32)) -> bool {
    let (cx, cy) = c.cursor();
    cx >= x && cx < x + SIZE && cy >= y && cy < y + SIZE
}

/// The buttons: a bordered square with three bars, lighter under the cursor.
pub fn draw(c: &dyn Container, canvas: &mut dyn Canvas) {
    for (at, _) in buttons(c) {
        let (x, y) = at;
        let face = if hit(c, at) { FACE_HOVERED } else { FACE };
        canvas.fill(x, y, SIZE, SIZE, EDGE);
        canvas.fill(x + 1.0, y + 1.0, SIZE - 2.0, SIZE - 2.0, face);
        for (i, w) in [5.0, 4.0, 3.0].into_iter().enumerate() {
            canvas.fill(x + 2.0, y + 2.0 + 2.0 * i as f32, w, 1.0, EDGE);
        }
    }
}

/// A left press on a button sorts its slots (true).
pub fn press(c: &mut dyn Container) -> bool {
    let Some((_, slots)) = buttons(c).into_iter().find(|(at, _)| hit(c, *at)) else {
        return false;
    };
    sort(c, &slots);
    true
}

/// The sort key: sorts the part under the cursor (the container when the cursor is over no
/// slot). False when the screen has nothing to sort.
pub fn key(c: &mut dyn Container, hovered: Option<SlotId>) -> bool {
    let buttons = buttons(c);
    let part = buttons
        .iter()
        .find(|(_, slots)| hovered.is_some_and(|h| slots.contains(&h)))
        .or(buttons.first());
    let Some((_, slots)) = part else {
        return false;
    };
    sort(c, slots);
    true
}

/// Sorts `slots` with ordinary left clicks (needs an empty hand).
fn sort(c: &mut dyn Container, slots: &[SlotId]) {
    if c.carried().is_some() {
        return;
    }
    let stacks: Vec<Option<Stack>> = slots.iter().map(|&s| c.stack(s)).collect();
    // Stacks that merge rank alike, so a kind's rank is its first slot's.
    let mut ranks: HashMap<u64, Rank> = HashMap::new();
    for (&slot, stack) in slots.iter().zip(&stacks) {
        if let Some(s) = stack
            && !ranks.contains_key(&s.kind)
        {
            ranks.insert(s.kind, c.order(slot));
        }
    }
    let rank = |s: &Stack| {
        ranks
            .get(&s.kind)
            .cloned()
            .unwrap_or((u32::MAX, String::new()))
    };
    for i in plan::plan(&stacks, &rank) {
        c.click(slots[i], Click::Pick { right: false });
    }
}
