//! Sorting's plan (#332, `docs/extras.md`): the left clicks that sort a run of slots, starting
//! and ending with an empty hand, as a player could click them, so a vanilla server accepts them.
//! Partial stacks of one item merge first; then the stacks go in rank order (creative tab order,
//! then name), full stacks before the partial one, empty slots last.
use horizon_platform::containers::Stack;

/// A stack's place in the order: its index in the creative tabs (items outside them after), then
/// its name.
pub type Rank = (u32, String);

/// A hand and slots under `AbstractContainerMenu.doClick`'s left `PICKUP`.
struct Sim {
    slots: Vec<Option<Stack>>,
    hand: Option<Stack>,
    clicks: Vec<usize>,
}
impl Sim {
    fn click(&mut self, i: usize) {
        self.clicks.push(i);
        let slot = self.slots[i].take();
        match (self.hand.take(), slot) {
            // Take the stack, or put the hand's down.
            (None, s) => self.hand = s,
            (Some(h), None) => self.slots[i] = Some(h),
            // The same item: as much as fits joins the slot.
            (Some(mut h), Some(mut s)) if s.kind == h.kind => {
                let n = s.max.saturating_sub(s.count).min(h.count);
                s.count += n;
                h.count -= n;
                self.slots[i] = Some(s);
                self.hand = (h.count > 0).then_some(h);
            }
            // Another item: they swap.
            (Some(h), Some(s)) => {
                self.slots[i] = Some(h);
                self.hand = Some(s);
            }
        }
    }
    fn partial(&self, i: usize) -> Option<&Stack> {
        self.slots[i].as_ref().filter(|s| s.count < s.max)
    }
}

/// The indices of `slots` to left-click, in order, to sort them. Empty when they are sorted
/// already, or when the clicks couldn't reach the order (stacks over their maximum).
pub fn plan(slots: &[Option<Stack>], rank: &dyn Fn(&Stack) -> Rank) -> Vec<usize> {
    let n = slots.len();
    let mut sim = Sim {
        slots: slots.to_vec(),
        hand: None,
        clicks: Vec::new(),
    };
    // Merge: each later partial stack of the item tops up the first one;
    // what doesn't fit goes back.
    for i in 0..n {
        for j in i + 1..n {
            let Some(s) = sim.partial(i) else {
                break;
            };
            if !sim.partial(j).is_some_and(|t| t.kind == s.kind) {
                continue;
            }
            sim.click(j);
            sim.click(i);
            if sim.hand.is_some() {
                sim.click(j);
            }
        }
    }
    let stacks: Vec<Stack> = sim.slots.iter().flatten().cloned().collect();
    let ranks: Vec<Rank> = stacks.iter().map(rank).collect();
    let mut order: Vec<usize> = (0..stacks.len()).collect();
    order.sort_by(|&a, &b| {
        ranks[a]
            .cmp(&ranks[b])
            .then(stacks[b].count.cmp(&stacks[a].count))
    });
    let target: Vec<Option<Stack>> = order
        .iter()
        .map(|&i| Some(stacks[i].clone()))
        .chain(std::iter::repeat(None))
        .take(n)
        .collect();
    // Order: each slot takes its stack from further on; what it held goes
    // where that stack was. A full stack clicked on the item's partial one
    // fills it and takes the rest, which comes to the same.
    for i in 0..n {
        if sim.slots[i] == target[i] {
            continue;
        }
        let Some(j) = (i + 1..n).find(|&j| sim.slots[j] == target[i]) else {
            return Vec::new();
        };
        sim.click(j);
        sim.click(i);
        if sim.hand.is_some() {
            sim.click(j);
        }
    }
    if sim.hand.is_some() || sim.slots != target {
        return Vec::new();
    }
    sim.clicks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ender pearls stack to 16, swords to 1, the rest to 64. Each item is its own kind.
    fn stack(item: &str, count: u32) -> Option<Stack> {
        let max = match item {
            "ender_pearl" | "snowball" => 16,
            "diamond_sword" => 1,
            _ => 64,
        };
        let kind = item
            .bytes()
            .fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(u64::from(b)));
        Some(Stack {
            item: format!("minecraft:{item}"),
            count,
            max,
            kind,
        })
    }
    fn rank(order: &'static [&'static str]) -> impl Fn(&Stack) -> Rank {
        move |s| {
            let id = s.item.trim_start_matches("minecraft:");
            let i = order
                .iter()
                .position(|o| *o == id)
                .map_or(u32::MAX, |i| i as u32);
            (i, id.to_owned())
        }
    }
    const ORDER: &[&str] = &["stone", "dirt", "diamond_sword", "ender_pearl"];

    #[test]
    fn merges_partial_stacks_then_orders_full_stacks_first() {
        let slots = vec![
            stack("ender_pearl", 3),
            None,
            stack("dirt", 40),
            stack("stone", 64),
            stack("dirt", 40),
            stack("ender_pearl", 15),
            stack("diamond_sword", 1),
            stack("zombie_head", 1),
        ];
        let clicks = plan(&slots, &rank(ORDER));
        let mut sim = Sim {
            slots: slots.clone(),
            hand: None,
            clicks: Vec::new(),
        };
        for i in clicks {
            sim.click(i);
        }
        assert_eq!(sim.hand, None);
        assert_eq!(
            sim.slots,
            vec![
                stack("stone", 64),
                stack("dirt", 64),
                stack("dirt", 16),
                stack("diamond_sword", 1),
                stack("ender_pearl", 16),
                stack("ender_pearl", 2),
                stack("zombie_head", 1),
                None,
            ]
        );
        // Sorted slots need no clicks.
        assert!(plan(&sim.slots, &rank(ORDER)).is_empty());
    }

    #[test]
    fn the_clicks_sort_a_chest_in_the_menu_engine() {
        // The planned clicks replayed as left PICKUPs on the engine's ChestMenu (checked against
        // the server jar) end in the planned order.
        use horizon_client_lib::{
            banner::Banners,
            brewing::Brewing,
            crafting::Recipes,
            creative::Names,
            enchant::Enchantments,
            items::{ItemStack, Items},
            menu::{Env, Input, Kind, Menu},
            stations::StationRecipes,
        };
        use horizon_world::loot::PackOverlay;
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        if !root.join("items.json").exists() {
            return;
        }
        let none = &PackOverlay::NONE;
        let items = Items::catalog(&root, none).unwrap();
        let env = Env {
            items: &items,
            recipes: &Recipes::load(&root, &items, none).unwrap(),
            brewing: &Brewing::load(&root, &items, none),
            banners: &Banners::load(&root),
            station_recipes: &StationRecipes::load(&root, &items, none),
            enchantments: &Enchantments::default(),
            names: &Names::default(),
            trade: None,
            maps: None,
            limited_crafting: false,
            known_recipes: None,
        };
        let mut menu = Menu::new(Kind::Chest(3));
        menu.creative = true;
        let engine = |s: &Option<Stack>| s.as_ref().map(|s| ItemStack::new(&s.item, s.count));
        let ours =
            |s: Option<&ItemStack>| stack(s?.item.trim_start_matches("minecraft:"), s?.count);
        let names = ["stone", "dirt", "ender_pearl", "oak_log", "snowball"];
        let order = rank(&["oak_log", "stone", "dirt"]);
        let mut seed = 7u32;
        for round in 0..40 {
            let slots: Vec<Option<Stack>> = (0..27)
                .map(|_| {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                    let item = names.get((seed >> 16) as usize % 8)?;
                    let top = stack(item, 1)?.max;
                    stack(item, 1 + (seed >> 8) % top)
                })
                .collect();
            for (i, s) in slots.iter().enumerate() {
                menu.set_slot(&env, i, engine(s));
            }
            for i in plan(&slots, &order) {
                menu.click(&env, i as i32, 0, Input::Pickup).unwrap();
            }
            assert_eq!(menu.carried, None, "round {round}");
            let after: Vec<Option<Stack>> = (0..27).map(|i| ours(menu.slot(i))).collect();
            let total = |v: &[Option<Stack>], item: &str| -> u32 {
                v.iter()
                    .flatten()
                    .filter(|s| s.item.ends_with(item))
                    .map(|s| s.count)
                    .sum()
            };
            for item in names {
                assert_eq!(total(&slots, item), total(&after, item), "round {round}");
            }
            assert!(plan(&after, &order).is_empty(), "round {round}");
            let ranks: Vec<Rank> = after.iter().flatten().map(&order).collect();
            assert!(ranks.is_sorted(), "round {round}");
        }
    }
}
