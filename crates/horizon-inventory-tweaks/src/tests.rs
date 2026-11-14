use super::*;
use horizon_platform::containers::{Click, Role, Slot, Stack};

/// Stone and dirt stack to 64, pearls to 16; each item is its own kind.
fn stack(item: &str, count: u32) -> Option<Stack> {
    let max = if item == "ender_pearl" { 16 } else { 64 };
    Some(Stack {
        item: format!("minecraft:{item}"),
        count,
        max,
        kind: item
            .bytes()
            .fold(0, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b))),
    })
}

/// A menu as vanilla's clicks treat it: a chest's 27 slots (ids 300..327, three rows), the main
/// inventory (9..36) and the hotbar (0..9), laid out as a chest screen draws them.
struct Menu {
    menu: &'static str,
    slots: Vec<(Slot, Option<Stack>)>,
    carried: Option<Stack>,
    spread: usize,
    cursor: (f32, f32),
    shift: bool,
    clicks: Vec<(u32, Click)>,
}

impl Menu {
    fn chest() -> Self {
        let mut slots = Vec::new();
        let mut add = |id: u32, role, x: f32, y: f32| {
            let slot = Slot {
                id: SlotId(id),
                role,
                x,
                y,
            };
            slots.push((slot, None));
        };
        for i in 0..9 {
            add(i, Role::Hotbar, 8.0 + 18.0 * i as f32, 142.0);
        }
        for i in 9..36 {
            let (row, col) = ((i - 9) / 9, (i - 9) % 9);
            add(
                i,
                Role::Inventory,
                8.0 + 18.0 * col as f32,
                84.0 + 18.0 * row as f32,
            );
        }
        for i in 0..27 {
            let (row, col) = (i / 9, i % 9);
            add(
                300 + i,
                Role::Storage,
                8.0 + 18.0 * col as f32,
                18.0 + 18.0 * row as f32,
            );
        }
        Menu {
            menu: "minecraft:generic_9x3",
            slots,
            carried: None,
            spread: 0,
            cursor: (0.0, 0.0),
            shift: false,
            clicks: Vec::new(),
        }
    }
    fn at(&mut self, id: u32) -> &mut Option<Stack> {
        &mut self.slots.iter_mut().find(|(s, _)| s.id.0 == id).unwrap().1
    }
    fn set(mut self, id: u32, stack: Option<Stack>) -> Self {
        *self.at(id) = stack;
        self
    }
    fn count(&mut self, id: u32) -> u32 {
        self.at(id).as_ref().map_or(0, |s| s.count)
    }
    fn role(&self, id: SlotId) -> Role {
        self.slots.iter().find(|(s, _)| s.id == id).unwrap().0.role
    }
    /// Puts `n` of the cursor's stack in `id` when it takes them all.
    fn put(&mut self, id: u32, n: u32) -> bool {
        let Some(hand) = self.carried.clone() else {
            return false;
        };
        let n = n.min(hand.count);
        let slot = self.at(id);
        if let Some(s) = slot {
            if s.kind != hand.kind || s.count + n > s.max {
                return false;
            }
            s.count += n;
        } else {
            *slot = Some(Stack {
                count: n,
                ..hand.clone()
            });
        }
        self.carried = (hand.count > n).then(|| Stack {
            count: hand.count - n,
            ..hand
        });
        true
    }
}

impl Container for Menu {
    fn menu(&self) -> &str {
        self.menu
    }
    fn slots(&self) -> Vec<Slot> {
        self.slots.iter().map(|(s, _)| *s).collect()
    }
    fn stack(&mut self, slot: SlotId) -> Option<Stack> {
        self.at(slot.0).clone()
    }
    fn carried(&self) -> Option<Stack> {
        self.carried.clone()
    }
    fn takes_one(&mut self, slot: SlotId, of: &Stack) -> bool {
        match self.at(slot.0) {
            None => true,
            Some(s) => s.kind == of.kind && s.count < s.max,
        }
    }
    /// The chest's slots go to the player's, hotbar first from the right; the player's to the
    /// chest: the first slots of the same item with room, then the first empty one.
    fn landing(&mut self, slot: SlotId) -> Vec<SlotId> {
        let Some(from) = self.at(slot.0).clone() else {
            return Vec::new();
        };
        let storage = self.role(slot) == Role::Storage;
        let mut other: Vec<_> = self
            .slots
            .iter()
            .filter(|(s, _)| (s.role == Role::Storage) != storage)
            .collect();
        if storage {
            // Hotbar first, from the right, as `ChestMenu.quickMoveStack` fills the player's.
            other.sort_by_key(|(s, _)| (s.role != Role::Hotbar, std::cmp::Reverse(s.id.0)));
        }
        let room = other.iter().filter(|(_, s)| {
            s.as_ref()
                .is_some_and(|s| s.kind == from.kind && s.count < s.max)
        });
        let empty = other.iter().filter(|(_, s)| s.is_none()).take(1);
        room.chain(empty).map(|(s, _)| s.id).collect()
    }
    fn click(&mut self, slot: SlotId, click: Click) -> bool {
        self.clicks.push((slot.0, click));
        match click {
            Click::Pick { right: false } => {
                let hand = self.carried.take();
                let s = self.at(slot.0).take();
                match (hand, s) {
                    (None, s) => self.carried = s,
                    (Some(h), None) => *self.at(slot.0) = Some(h),
                    (Some(mut h), Some(mut s)) if s.kind == h.kind => {
                        let n = s.max.saturating_sub(s.count).min(h.count);
                        s.count += n;
                        h.count -= n;
                        *self.at(slot.0) = Some(s);
                        self.carried = (h.count > 0).then_some(h);
                    }
                    (Some(h), Some(s)) => {
                        *self.at(slot.0) = Some(h);
                        self.carried = Some(s);
                    }
                }
                true
            }
            Click::QuickMove => {
                let Some(s) = self.at(slot.0).take() else {
                    return false;
                };
                let to = self.empty_across(slot);
                *self.at(to) = Some(s);
                true
            }
            Click::Put { one } => {
                let n = if one { 1 } else { u32::MAX };
                self.put(slot.0, n)
            }
            _ => false,
        }
    }
    fn spread(&self) -> usize {
        self.spread
    }
    fn cursor(&self) -> (f32, f32) {
        self.cursor
    }
    fn shift(&self) -> bool {
        self.shift
    }
    /// Stone, then dirt, then the rest by name.
    fn order(&mut self, slot: SlotId) -> (u32, String) {
        let item = self.at(slot.0).as_ref().map(|s| s.item.clone()).unwrap();
        let rank = ["minecraft:stone", "minecraft:dirt"]
            .iter()
            .position(|i| *i == item);
        (rank.map_or(u32::MAX, |r| r as u32), item)
    }
}

impl Menu {
    /// Where a quick move from `slot` puts its stack whole: the first empty slot across.
    fn empty_across(&self, slot: SlotId) -> u32 {
        let storage = self.role(slot) == Role::Storage;
        let across =
            |(t, v): &&(Slot, Option<Stack>)| (t.role == Role::Storage) != storage && v.is_none();
        self.slots.iter().find(across).unwrap().0.id.0
    }
}

#[test]
fn a_right_drag_places_one_in_each_slot_entered_once() {
    let mut menu = Menu::chest();
    menu.carried = stack("stone", 3);
    let mut tweaks = tweaks::Tweaks::default();
    assert!(tweaks.press(true, &mut menu, Some(SlotId(300)), true));
    assert!(tweaks.drag(true, &mut menu, Some(SlotId(301))));
    assert!(
        tweaks.drag(true, &mut menu, Some(SlotId(300))),
        "visited: nothing more"
    );
    tweaks.release();
    assert_eq!((menu.count(300), menu.count(301)), (1, 1));
    assert_eq!(menu.carried.as_ref().map(|s| s.count), Some(1));
    assert!(
        menu.clicks
            .iter()
            .all(|(_, c)| *c == Click::Put { one: true })
    );
}

#[test]
fn off_or_in_a_menu_with_costs_the_screen_stays_vanillas() {
    for (on, name) in [(false, "minecraft:generic_9x3"), (true, "minecraft:anvil")] {
        let mut menu = Menu::chest();
        menu.menu = name;
        menu.carried = stack("stone", 3);
        let mut tweaks = tweaks::Tweaks::default();
        assert!(
            !tweaks.press(on, &mut menu, Some(SlotId(300)), true),
            "{name}"
        );
        assert!(!tweaks.drag(on, &mut menu, Some(SlotId(301))), "{name}");
        assert!(
            !tweaks.wheel(on, &mut menu, Some(SlotId(300)), -1.0),
            "{name}"
        );
        assert!(menu.clicks.is_empty(), "{name}");
    }
}

#[test]
fn a_left_drag_gathers_the_held_item_until_vanillas_drag_spreads() {
    let mut menu = Menu::chest()
        .set(301, stack("stone", 5))
        .set(302, stack("dirt", 5))
        .set(303, stack("stone", 7));
    menu.carried = stack("stone", 10);
    let mut tweaks = tweaks::Tweaks::default();
    // The press is vanilla's; the tweaks watch.
    assert!(!tweaks.press(true, &mut menu, Some(SlotId(300)), false));
    assert!(
        !tweaks.drag(true, &mut menu, Some(SlotId(302))),
        "another item"
    );
    assert!(tweaks.drag(true, &mut menu, Some(SlotId(301))));
    assert!(tweaks.drag(true, &mut menu, Some(SlotId(303))));
    assert_eq!(menu.carried.as_ref().map(|s| s.count), Some(22));
    assert_eq!(
        (menu.count(301), menu.count(302), menu.count(303)),
        (0, 5, 0)
    );
    // Once vanilla's drag spread over a second slot, it stays vanilla's.
    let mut menu = Menu::chest().set(301, stack("stone", 5));
    menu.carried = stack("stone", 10);
    menu.spread = 2;
    let mut tweaks = tweaks::Tweaks::default();
    tweaks.press(true, &mut menu, Some(SlotId(300)), false);
    assert!(!tweaks.drag(true, &mut menu, Some(SlotId(301))));
    assert!(menu.clicks.is_empty());
}

#[test]
fn shift_dragging_an_empty_hand_moves_each_slot_entered() {
    let mut menu = Menu::chest()
        .set(300, stack("stone", 5))
        .set(301, stack("dirt", 5));
    menu.shift = true;
    let mut tweaks = tweaks::Tweaks::default();
    assert!(!tweaks.press(true, &mut menu, Some(SlotId(300)), false));
    assert!(tweaks.drag(true, &mut menu, Some(SlotId(301))));
    assert_eq!(menu.clicks, [(301, Click::QuickMove)]);
    assert_eq!(menu.count(301), 0);
}

#[test]
fn the_wheel_moves_one_item_each_notch_and_brings_one_back() {
    let mut menu = Menu::chest().set(300, stack("stone", 5));
    let mut tweaks = tweaks::Tweaks::default();
    // Two notches down, in fractions as a trackpad sends them.
    for _ in 0..4 {
        assert!(tweaks.wheel(true, &mut menu, Some(SlotId(300)), -0.5));
    }
    assert_eq!(menu.count(300), 3);
    // A quick move of the chest's stack goes to the hotbar's right end first.
    assert_eq!(menu.count(8), 2);
    assert_eq!(menu.carried, None);
    // Up over the chest's slot brings one back from the player's side.
    tweaks.wheel(true, &mut menu, Some(SlotId(300)), 1.0);
    assert_eq!((menu.count(300), menu.count(8)), (4, 1));
    // Not with a stack on the cursor.
    menu.carried = stack("dirt", 1);
    assert!(!tweaks.wheel(true, &mut menu, Some(SlotId(300)), -1.0));
}

#[test]
fn the_buttons_and_the_key_sort_their_part_with_left_clicks() {
    let mut menu = Menu::chest()
        .set(300, stack("dirt", 40))
        .set(302, stack("stone", 3))
        .set(303, stack("dirt", 40))
        .set(9, stack("dirt", 1))
        .set(10, stack("stone", 1));
    // The chest's button is over its last column, on its title line.
    menu.cursor = (8.0 + 18.0 * 8.0 + 8.0 + 4.0, 18.0 - 13.0 + 4.0);
    assert!(sort::press(&mut menu));
    let chest: Vec<u32> = (300..304).map(|i| menu.count(i)).collect();
    assert_eq!(chest, [3, 64, 16, 0]);
    assert_eq!(menu.at(300).as_ref().unwrap().item, "minecraft:stone");
    assert!(
        menu.clicks
            .iter()
            .all(|(_, c)| *c == Click::Pick { right: false })
    );
    assert_eq!(menu.count(9), 1, "the inventory is another part");
    // The key sorts the part under the cursor: the inventory.
    assert!(sort::key(&mut menu, Some(SlotId(20))));
    assert_eq!(menu.at(9).as_ref().unwrap().item, "minecraft:stone");
    // Elsewhere, the press is the screen's.
    menu.cursor = (0.0, 0.0);
    assert!(!sort::press(&mut menu));
}

#[test]
fn the_settings_load_show_change_and_save_per_seat() {
    use horizon_hooks::engine::set_seat;
    // A seat no other test plays, as the tests share `HOOKS`.
    set_seat(6);
    assert!(SETTINGS.is_default() && SETTINGS.owns("sortKey") && !SETTINGS.owns("zoom"));
    SETTINGS.load("mouseTweaks", "true");
    SETTINGS.load("sortKey", "not a key");
    let mut rows = Vec::new();
    SECTION.rows(&mut rows);
    assert_eq!(
        rows[0],
        Row::Toggle {
            label: "horizon.extras.mouse_tweaks",
            on: true
        }
    );
    assert!(SECTION.live(1) && !SECTION.live(3), "sorting is off");
    SECTION.act(2, Act::Press);
    SECTION.act(3, Act::Bind("key.keyboard.r".into()));
    assert!(SECTION.live(3));
    let mut saved = Vec::new();
    SETTINGS.save(&mut |key, value| saved.push(format!("{key}:{value}")));
    assert_eq!(
        saved,
        [
            "mouseTweaks:true",
            "mouseTweaksWheel:true",
            "sort:true",
            "sortKey:key.keyboard.r"
        ]
    );
    set_seat(5);
    assert!(SETTINGS.is_default(), "another player's are theirs");
    set_seat(0);
}
