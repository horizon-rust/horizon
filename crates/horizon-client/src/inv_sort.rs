//! Horizon's sorting extra (#332, `docs/extras.md`): the left clicks that
//! sort a run of slots, starting and ending with an empty hand, as a player
//! could click them, so a vanilla server accepts them. Partial stacks of
//! one item merge first; then the stacks go in rank order (creative tab
//! order, then name), full stacks before the partial one, empty slots last.
use crate::bundle::{Contents, can_be_in_bundle};
use crate::items::{ItemStack, Items};

/// A stack's place in the order: its index in the creative tabs (items
/// outside them after), then its name.
pub type Rank = (usize, String);

/// Fill bundles with stacks to free inventory slots, then sort the remaining slots.
/// Bundles stay in their slots: clicking them as ordinary stacks would instead
/// invoke the bundle's insertion behaviour.
pub fn plan_with_bundles(
    slots: &[Option<ItemStack>],
    items: &Items,
    rank: &dyn Fn(&ItemStack) -> Rank,
    max: &dyn Fn(&ItemStack) -> u32,
) -> Vec<usize> {
    let mut state = slots.to_vec();
    let mut clicks = Vec::new();
    let bundles: Vec<usize> = state
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.as_ref().and_then(|s| Contents::of(items, s)).map(|_| i))
        .collect();
    if bundles.is_empty() {
        return plan(slots, rank, max);
    }
    // Consolidate compatible stacks before allocating space in bundles. Without
    // this, moving several partial stacks of one kind can appear better than
    // moving distinct kinds, even when those stacks would merge outside.
    let sortable: Vec<usize> = (0..slots.len()).filter(|i| !bundles.contains(i)).collect();
    let views: Vec<_> = sortable.iter().map(|&i| state[i].clone()).collect();
    let initial_clicks = plan(&views, rank, max);
    let mut initial = Sim {
        slots: views,
        hand: None,
        max,
        clicks: Vec::new(),
    };
    for i in initial_clicks {
        initial.click(i);
        clicks.push(sortable[i]);
    }
    for (i, item) in sortable.iter().zip(initial.slots) {
        state[*i] = item;
    }
    let empty = ItemStack::new("minecraft:bundle", 1);
    let Some(empty_contents) = Contents::of(items, &empty) else {
        return Vec::new();
    };
    let mut candidates: Vec<usize> = state
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let s = s.as_ref()?;
            (Contents::of(items, s).is_none() && can_be_in_bundle(s)).then_some(i)
        })
        .collect();
    candidates.sort_by(|&a, &b| {
        let a = state[a].as_ref().unwrap();
        let b = state[b].as_ref().unwrap();
        let ar = empty_contents.room_for(items, a).max(1);
        let br = empty_contents.room_for(items, b).max(1);
        (u64::from(a.count) * u64::from(br))
            .cmp(&(u64::from(b.count) * u64::from(ar)))
            .then_with(|| rank(a).cmp(&rank(b)))
    });
    // Search whole-stack assignments first: a full transfer frees a slot.
    // The node limit bounds work in large inventories; the best plan found so
    // far is still usable when the limit is reached.
    struct Search<'a> {
        items: &'a Items,
        stacks: &'a [Option<ItemStack>],
        candidates: &'a [usize],
        remaining: usize,
        best: Vec<Option<usize>>,
        best_score: (usize, usize),
    }
    impl Search<'_> {
        fn visit(
            &mut self,
            i: usize,
            bags: &mut [ItemStack],
            chosen: &mut [Option<usize>],
            diversity: usize,
            count: usize,
        ) {
            if (count, diversity) > self.best_score {
                self.best_score = (count, diversity);
                self.best.clone_from_slice(chosen);
            }
            if self.remaining == 0
                || i == self.candidates.len()
                || (
                    count + self.candidates.len() - i,
                    diversity + self.candidates.len() - i,
                ) <= self.best_score
            {
                return;
            }
            self.remaining -= 1;
            let stack = self.stacks[self.candidates[i]].as_ref().unwrap();
            let new_kind = !bags.iter().any(|bag| {
                Contents::of(self.items, bag)
                    .unwrap()
                    .items()
                    .iter()
                    .any(|s| s.same_item_same_components(stack))
            });
            for bag in 0..bags.len() {
                let mut contents = Contents::of(self.items, &bags[bag]).unwrap();
                if contents.room_for(self.items, stack) < stack.count {
                    continue;
                }
                let before = bags[bag].clone();
                let mut transferred = stack.clone();
                contents.try_insert(self.items, &mut transferred);
                contents.store(&mut bags[bag]);
                chosen[i] = Some(bag);
                self.visit(
                    i + 1,
                    bags,
                    chosen,
                    diversity + usize::from(new_kind),
                    count + 1,
                );
                chosen[i] = None;
                bags[bag] = before;
            }
            self.visit(i + 1, bags, chosen, diversity, count);
        }
    }
    let mut search = Search {
        items,
        stacks: &state,
        candidates: &candidates,
        remaining: 4096,
        best: vec![None; candidates.len()],
        best_score: (0, 0),
    };
    let mut bags: Vec<_> = bundles.iter().map(|&i| state[i].clone().unwrap()).collect();
    search.visit(0, &mut bags, &mut vec![None; candidates.len()], 0, 0);
    let assignment = search.best;

    for (bag_index, &b) in bundles.iter().enumerate() {
        let mut contents = Contents::of(items, state[b].as_ref().unwrap()).unwrap();
        let mut selected = Vec::new();
        for (candidate, &slot) in candidates.iter().enumerate() {
            if assignment[candidate] != Some(bag_index) {
                continue;
            }
            let stack = state[slot].as_mut().unwrap();
            contents.try_insert(items, stack);
            state[slot] = None;
            selected.push(slot);
        }
        if !selected.is_empty() {
            contents.store(state[b].as_mut().unwrap());
            clicks.push(b);
            clicks.extend(selected);
            clicks.push(b);
        }
    }
    // Use residual capacity for a new kind, even when only part of its stack
    // fits. Keep full stacks intact unless they can be transferred completely.
    for &slot in &candidates {
        let Some(stack) = state[slot].as_ref() else {
            continue;
        };
        if stack.count >= max(stack) {
            continue;
        }
        if bundles.iter().any(|&b| {
            Contents::of(items, state[b].as_ref().unwrap())
                .unwrap()
                .items()
                .iter()
                .any(|s| s.same_item_same_components(stack))
        }) {
            continue;
        }
        for &b in &bundles {
            let mut contents = Contents::of(items, state[b].as_ref().unwrap()).unwrap();
            if contents.room_for(items, stack) == 0 {
                continue;
            }
            let mut remaining = state[slot].take().unwrap();
            contents.try_insert(items, &mut remaining);
            state[slot] = (remaining.count > 0).then_some(remaining);
            contents.store(state[b].as_mut().unwrap());
            clicks.extend([b, slot, b]);
            break;
        }
    }
    // Once no new kind can use the space, merge a whole remaining stack into
    // an existing kind where it fits, freeing another inventory slot.
    for &slot in &candidates {
        let Some(stack) = state[slot].as_ref() else {
            continue;
        };
        for &b in &bundles {
            let mut contents = Contents::of(items, state[b].as_ref().unwrap()).unwrap();
            if !contents
                .items()
                .iter()
                .any(|s| s.same_item_same_components(stack))
                || contents.room_for(items, stack) < stack.count
            {
                continue;
            }
            let mut moved = state[slot].take().unwrap();
            contents.try_insert(items, &mut moved);
            contents.store(state[b].as_mut().unwrap());
            clicks.extend([b, slot, b]);
            break;
        }
    }
    let sortable: Vec<usize> = (0..slots.len()).filter(|i| !bundles.contains(i)).collect();
    let views: Vec<_> = sortable.iter().map(|&i| state[i].clone()).collect();
    clicks.extend(plan(&views, rank, max).into_iter().map(|i| sortable[i]));
    clicks
}

/// A hand and slots under `AbstractContainerMenu.doClick`'s left `PICKUP`.
struct Sim<'a> {
    slots: Vec<Option<ItemStack>>,
    hand: Option<ItemStack>,
    max: &'a dyn Fn(&ItemStack) -> u32,
    clicks: Vec<usize>,
}
impl Sim<'_> {
    fn click(&mut self, i: usize) {
        self.clicks.push(i);
        let slot = self.slots[i].take();
        match (self.hand.take(), slot) {
            // Take the stack, or put the hand's down.
            (None, s) => self.hand = s,
            (Some(h), None) => self.slots[i] = Some(h),
            // The same item: as much as fits joins the slot.
            (Some(mut h), Some(mut s)) if s.same_item_same_components(&h) => {
                let n = (self.max)(&s).saturating_sub(s.count).min(h.count);
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
    fn partial(&self, i: usize) -> Option<&ItemStack> {
        self.slots[i].as_ref().filter(|s| s.count < (self.max)(s))
    }
}

/// The indices of `slots` to left-click, in order, to sort them. Empty
/// when they are sorted already, or when the clicks couldn't reach the
/// order (stacks over their maximum).
pub fn plan(
    slots: &[Option<ItemStack>],
    rank: &dyn Fn(&ItemStack) -> Rank,
    max: &dyn Fn(&ItemStack) -> u32,
) -> Vec<usize> {
    let n = slots.len();
    let mut sim = Sim {
        slots: slots.to_vec(),
        hand: None,
        max,
        clicks: Vec::new(),
    };
    // Merge: each later partial stack of the item tops up the first one;
    // what doesn't fit goes back.
    for i in 0..n {
        for j in i + 1..n {
            let Some(s) = sim.partial(i) else {
                break;
            };
            if !sim
                .partial(j)
                .is_some_and(|t| t.same_item_same_components(s))
            {
                continue;
            }
            sim.click(j);
            sim.click(i);
            if sim.hand.is_some() {
                sim.click(j);
            }
        }
    }
    let stacks: Vec<ItemStack> = sim.slots.iter().flatten().cloned().collect();
    let ranks: Vec<Rank> = stacks.iter().map(rank).collect();
    let mut order: Vec<usize> = (0..stacks.len()).collect();
    order.sort_by(|&a, &b| {
        ranks[a]
            .cmp(&ranks[b])
            .then(stacks[b].count.cmp(&stacks[a].count))
    });
    let target: Vec<Option<ItemStack>> = order
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
    use crate::menu::{Input, Kind};

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack {
            item: format!("minecraft:{item}").into(),
            count,
            damage: 0,
            extra: None,
        })
    }
    fn rank(order: &'static [&'static str]) -> impl Fn(&ItemStack) -> Rank {
        move |s| {
            let id = s.item.trim_start_matches("minecraft:");
            let i = order.iter().position(|o| *o == id).unwrap_or(usize::MAX);
            (i, id.to_owned())
        }
    }
    fn max(s: &ItemStack) -> u32 {
        match &*s.item {
            "minecraft:ender_pearl" => 16,
            "minecraft:diamond_sword" => 1,
            _ => 64,
        }
    }
    const ORDER: &[&str] = &["stone", "dirt", "diamond_sword", "ender_pearl"];

    #[test]
    fn bundle_allocation_scenarios() {
        type Scenario = (
            &'static str,
            &'static [u32],
            &'static [(&'static str, u32)],
            usize,
            bool,
        );
        let scenarios: &[Scenario] = &[
            (
                "no bundles",
                &[],
                &[("stone", 20), ("stone", 30), ("dirt", 4)],
                0,
                false,
            ),
            (
                "mushrooms and diamonds",
                &[64],
                &[("red_mushroom", 2), ("brown_mushroom", 2), ("diamond", 64)],
                2,
                false,
            ),
            (
                "one bundle and a full diamond stack",
                &[64],
                &[("diamond", 64)],
                1,
                false,
            ),
            (
                "consolidation before allocation",
                &[64],
                &[
                    ("cobblestone", 10),
                    ("cobblestone", 10),
                    ("cobblestone", 10),
                    ("cobblestone", 10),
                    ("cobblestone", 10),
                    ("cobblestone", 10),
                    ("dirt", 20),
                    ("sand", 20),
                    ("gravel", 20),
                ],
                4,
                false,
            ),
            (
                "full stacks",
                &[64],
                &[("diamond", 64), ("dirt", 64)],
                1,
                false,
            ),
            (
                "smallest first",
                &[6],
                &[
                    ("red_mushroom", 1),
                    ("brown_mushroom", 2),
                    ("dirt", 3),
                    ("cobblestone", 4),
                ],
                3,
                false,
            ),
            (
                "prioritize freed slots",
                &[2, 2],
                &[
                    ("red_mushroom", 1),
                    ("brown_mushroom", 1),
                    ("dirt", 2),
                    ("sand", 2),
                ],
                3,
                false,
            ),
            ("partial transfer", &[2], &[("dirt", 3)], 1, false),
            (
                "full bundle",
                &[0],
                &[("dirt", 1), ("red_mushroom", 1)],
                0,
                false,
            ),
            (
                "mixed stack capacities",
                &[64],
                &[
                    ("ender_pearl", 4),
                    ("red_mushroom", 2),
                    ("brown_mushroom", 2),
                ],
                3,
                false,
            ),
            (
                "two bundles",
                &[10, 10],
                &[
                    ("red_mushroom", 2),
                    ("brown_mushroom", 2),
                    ("dirt", 2),
                    ("cobblestone", 3),
                    ("sand", 3),
                    ("gravel", 4),
                    ("clay_ball", 4),
                ],
                7,
                false,
            ),
            (
                "existing kind",
                &[64],
                &[("red_mushroom", 3), ("brown_mushroom", 2)],
                1,
                true,
            ),
            (
                "large and small partial stacks",
                &[64],
                &[("red_mushroom", 63), ("brown_mushroom", 2)],
                2,
                false,
            ),
        ];
        crate::menu::tests::with_menu(Kind::Chest(3), |env, _| {
            for &(name, capacities, inputs, expected, existing) in scenarios {
                let mut menu = crate::menu::Menu::new(Kind::Chest(3));
                let mut slots = vec![None; 27];
                let mut before = std::collections::BTreeMap::<String, u32>::new();
                for (i, &capacity) in capacities.iter().enumerate() {
                    let mut bundle = stack("bundle", 1).unwrap();
                    if capacity < 64 {
                        bundle
                            .data_mut()
                            .bundle_contents
                            .push(stack("stone", 64 - capacity).unwrap());
                        *before.entry("minecraft:stone".into()).or_default() += 64 - capacity;
                    }
                    if existing {
                        bundle
                            .data_mut()
                            .bundle_contents
                            .push(stack("red_mushroom", 2).unwrap());
                        *before.entry("minecraft:red_mushroom".into()).or_default() += 2;
                    }
                    slots[i] = Some(bundle);
                }
                for (i, &(id, count)) in inputs.iter().enumerate() {
                    let item = stack(id, count).unwrap();
                    *before.entry(item.item.to_string()).or_default() += count;
                    slots[capacities.len() + i] = Some(item);
                }
                for (i, item) in slots.iter().enumerate() {
                    menu.set_slot(env, i, item.clone());
                }
                let max = |s: &ItemStack| env.items.get(&s.item).map_or(64, |i| i.max_stack);
                for i in plan_with_bundles(&slots, env.items, &rank(ORDER), &max) {
                    menu.click(env, i as i32, 0, Input::Pickup).unwrap();
                }
                let mut after = std::collections::BTreeMap::<String, u32>::new();
                let mut added = 0;
                for i in 0..27 {
                    if let Some(item) = menu.slot(i) {
                        if let Some(contents) = Contents::of(env.items, item) {
                            added += contents.items().len()
                                - usize::from(capacities[i] < 64)
                                - usize::from(existing);
                            for inner in contents.items() {
                                *after.entry(inner.item.to_string()).or_default() += inner.count;
                            }
                        } else {
                            *after.entry(item.item.to_string()).or_default() += item.count;
                        }
                    }
                }
                assert_eq!(after, before, "items changed: {name}");
                assert_eq!(menu.carried, None, "carried stack: {name}");
                assert_eq!(added, expected, "diversity: {name}");
                if name == "consolidation before allocation" || name == "prioritize freed slots" {
                    assert_eq!(
                        (capacities.len()..27)
                            .filter(|&i| menu.slot(i).is_some())
                            .count(),
                        1,
                        "must leave one combined stack outside"
                    );
                }
                if name == "existing kind" {
                    assert!((capacities.len()..27).all(|i| {
                        menu.slot(i)
                            .is_none_or(|s| s.item.as_ref() != "minecraft:red_mushroom")
                    }));
                }
                let after_slots: Vec<_> = (0..27).map(|i| menu.slot(i).cloned()).collect();
                assert!(
                    plan_with_bundles(&after_slots, env.items, &rank(ORDER), &max).is_empty(),
                    "not stable: {name}"
                );
            }
        });
    }

    #[test]
    fn bundle_sort_uses_real_menu_clicks_and_preserves_full_stacks() {
        crate::menu::tests::with_menu(Kind::Chest(3), |env, menu| {
            let mut slots = vec![None; 27];
            slots[0] = stack("bundle", 1);
            slots[1] = stack("diamond", 64);
            slots[2] = stack("red_mushroom", 2);
            slots[3] = stack("brown_mushroom", 2);
            for (i, s) in slots.iter().enumerate() {
                menu.set_slot(env, i, s.clone());
            }
            let max = |s: &ItemStack| env.items.get(&s.item).map_or(64, |i| i.max_stack);
            let clicks = plan_with_bundles(&slots, env.items, &rank(ORDER), &max);
            for i in clicks {
                menu.click(env, i as i32, 0, Input::Pickup).unwrap();
            }
            assert_eq!(menu.carried, None);
            let bundle = menu.slot(0).unwrap();
            let contents = Contents::of(env.items, bundle).unwrap();
            assert_eq!(contents.items().len(), 2);
            assert!(
                contents
                    .items()
                    .iter()
                    .any(|s| s.item.ends_with("red_mushroom") && s.count == 2)
            );
            assert!(
                contents
                    .items()
                    .iter()
                    .any(|s| s.item.ends_with("brown_mushroom") && s.count == 2)
            );
            assert!((1..27).any(|i| menu.slot(i) == stack("diamond", 64).as_ref()));
            let after: Vec<_> = (0..27).map(|i| menu.slot(i).cloned()).collect();
            assert!(plan_with_bundles(&after, env.items, &rank(ORDER), &max).is_empty());
        });
    }

    #[test]
    fn partially_filled_bundles_keep_contents_and_reject_duplicates_and_shulkers() {
        crate::menu::tests::with_menu(Kind::Chest(3), |env, menu| {
            let original = stack("stone", 62).unwrap();
            let mut first = stack("bundle", 1).unwrap();
            first.data_mut().bundle_contents.push(original.clone());
            let second = first.clone();
            let mut slots = vec![None; 27];
            slots[0] = Some(first);
            slots[1] = Some(second);
            slots[2] = stack("red_mushroom", 2);
            slots[3] = stack("brown_mushroom", 2);
            slots[4] = stack("stone", 1);
            slots[5] = stack("shulker_box", 1);
            let max = |s: &ItemStack| env.items.get(&s.item).map_or(64, |i| i.max_stack);
            for (i, s) in slots.iter().enumerate() {
                menu.set_slot(env, i, s.clone());
            }
            for i in plan_with_bundles(&slots, env.items, &rank(ORDER), &max) {
                menu.click(env, i as i32, 0, Input::Pickup).unwrap();
            }
            assert_eq!(menu.carried, None);
            let mut mushrooms = Vec::new();
            for i in 0..2 {
                let contents = Contents::of(env.items, menu.slot(i).unwrap()).unwrap();
                assert_eq!(contents.items().len(), 2);
                assert!(contents.items().contains(&original));
                mushrooms.extend(
                    contents
                        .items()
                        .iter()
                        .filter(|s| s.item.ends_with("mushroom"))
                        .cloned(),
                );
            }
            assert_eq!(mushrooms.len(), 2);
            assert!(
                mushrooms
                    .iter()
                    .any(|s| s.item.ends_with("red_mushroom") && s.count == 2)
            );
            assert!(
                mushrooms
                    .iter()
                    .any(|s| s.item.ends_with("brown_mushroom") && s.count == 2)
            );
            assert!((2..27).any(|i| menu.slot(i) == stack("stone", 1).as_ref()));
            assert!((2..27).any(|i| menu.slot(i) == stack("shulker_box", 1).as_ref()));
            let after: Vec<_> = (0..27).map(|i| menu.slot(i).cloned()).collect();
            assert!(plan_with_bundles(&after, env.items, &rank(ORDER), &max).is_empty());
        });
    }

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
        let clicks = plan(&slots, &rank(ORDER), &max);
        let mut sim = Sim {
            slots: slots.clone(),
            hand: None,
            max: &max,
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
        assert!(plan(&sim.slots, &rank(ORDER), &max).is_empty());
    }

    #[test]
    fn the_clicks_sort_a_chest_in_the_menu_engine() {
        // The planned clicks replayed as left PICKUPs on a ChestMenu: the
        // engine (checked against the server jar) ends in the planned order.
        crate::menu::tests::with_menu(Kind::Chest(3), |env, menu| {
            let max = |s: &ItemStack| env.items.get(&s.item).map_or(64, |i| i.max_stack);
            let names = ["stone", "dirt", "ender_pearl", "oak_log", "snowball"];
            let mut seed = 7u32;
            for round in 0..40 {
                let slots: Vec<Option<ItemStack>> = (0..27)
                    .map(|_| {
                        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                        let pick = (seed >> 16) as usize % 8;
                        let item = names.get(pick)?;
                        let top = if *item == "ender_pearl" || *item == "snowball" {
                            16
                        } else {
                            64
                        };
                        stack(item, 1 + (seed >> 8) % top)
                    })
                    .collect();
                for (i, s) in slots.iter().enumerate() {
                    menu.set_slot(env, i, s.clone());
                }
                let clicks = plan(&slots, &rank(&["oak_log", "stone", "dirt"]), &max);
                for i in &clicks {
                    menu.click(env, *i as i32, 0, Input::Pickup).unwrap();
                }
                assert_eq!(menu.carried, None, "round {round}");
                let after: Vec<Option<ItemStack>> =
                    (0..27).map(|i| menu.slot(i).cloned()).collect();
                let total = |v: &[Option<ItemStack>], item: &str| -> u32 {
                    v.iter()
                        .flatten()
                        .filter(|s| s.item.ends_with(item))
                        .map(|s| s.count)
                        .sum()
                };
                for item in names {
                    assert_eq!(total(&slots, item), total(&after, item), "round {round}");
                }
                assert!(plan(&after, &rank(&["oak_log", "stone", "dirt"]), &max).is_empty());
                let items: Vec<&str> = after.iter().flatten().map(|s| &*s.item).collect();
                let mut sorted = items.clone();
                sorted.sort_by_key(|id| {
                    rank(&["oak_log", "stone", "dirt"])(&stack(&id[10..], 1).unwrap())
                });
                assert_eq!(items, sorted, "round {round}");
            }
        });
    }
}
