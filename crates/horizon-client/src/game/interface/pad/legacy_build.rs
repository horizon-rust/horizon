//! The controller crafting screen's builders (`legacy_build.rs` in the library): the mode tabs
//! after Crafting, for the recipes built from what's put in the grid.
//!
//! The row lists the builder's templates; the grid shows the staged items, and with the focus
//! there A and X step a cell through what the inventory has that fits it. The result is the
//! game's own match of the staged grid. A crafts it, Y as many as the items allow: the items go
//! into the grid by vanilla's clicks (pick up, right-click one into the cell, put the rest
//! back), the result is quick-moved out, and anything left in the grid goes back. No recipe-book
//! packet places these recipes, so this is what a player does by hand, one click a frame, and on
//! someone else's server only with Quick Craft on Servers.
use super::super::Game;
use horizon_client::{
    gamepad::{self, Button},
    items::ItemStack,
    legacy_build::{self, Kind, Template},
    legacy_craft_screen::{self as screen, Area, Cell, Hit, RecipeSlot, View},
};
use std::{
    collections::{HashMap, VecDeque},
    time::{Duration, Instant},
};

/// The gap between two clicks on any server but the window's own: one a game tick.
const CLICK_GAP: Duration = Duration::from_millis(50);
/// How long the result may take to show after the last item went in.
const RESULT_WAIT: Duration = Duration::from_secs(2);
/// The most crafts one Y makes.
const MOST: u32 = 64;
/// The crafting grid's first slot and its result, as the menu numbers them.
const GRID: usize = 100;
const RESULT: usize = 200;

/// The builders' state between frames.
#[derive(Default)]
pub(in crate::game) struct Build {
    /// The templates, and the recipe count they were made from.
    templates: Option<(usize, Vec<Template>)>,
    /// What's staged in each template's cells, by recipe id.
    staged: HashMap<String, Vec<Option<ItemStack>>>,
    /// Each builder's row entry the player picked (none: the first that can be made), and the
    /// variant (template) picked of it (none: its first that can be made).
    selected: HashMap<Kind, (Option<usize>, Option<usize>)>,
    run: Option<Run>,
}

impl Build {
    /// Ends a running craft where it is: no more clicks go out.
    pub(in crate::game) fn end_run(&mut self) {
        self.run = None;
    }
}

/// The selected builder as it stands.
struct Now {
    ours: Vec<Template>,
    groups: Vec<Vec<usize>>,
    entry: usize,
    variant: usize,
    template: Template,
    staged: Vec<Option<ItemStack>>,
}

/// A craft under way: the clicks still to make.
struct Run {
    recipe: String,
    /// The menu it was started on: on any other, it ends (`legacy_build_poll`).
    menu: i32,
    steps: VecDeque<Step>,
    next_at: Instant,
    /// The menu's result slot, and its input slots (what Clear empties): the crafting grid's.
    result: usize,
    inputs: Vec<usize>,
    /// Crafts still asked for after this one.
    more: u32,
}

#[derive(Clone, Debug)]
enum Step {
    /// One staged item into a grid slot (three clicks, found in the inventory when it's due).
    Put { stack: ItemStack, cell: usize },
    Click {
        slot: usize,
        right: bool,
        shift: bool,
    },
    /// Until the server shows a result, or the wait runs out.
    Result { until: Option<Instant> },
    /// The result, quick-moved into the inventory.
    Take,
    /// Whatever stays in the grid, quick-moved back.
    Clear,
    /// The inputs are empty, or the run ends here (a full inventory left something in them,
    /// which a Put would swap with what it carries).
    Empty,
}

impl Game<'_> {
    /// The builder the mode tabs have selected: none on Crafting.
    pub(in crate::game) fn legacy_builder(&self) -> Option<Kind> {
        self.pad
            .legacy
            .mode
            .checked_sub(1)
            .and_then(|i| Kind::ALL.get(i))
            .copied()
    }

    /// The mode tabs' icons: Crafting, then each builder.
    pub(in crate::game) fn legacy_mode_icons(&self) -> Vec<ItemStack> {
        std::iter::once("minecraft:crafting_table")
            .chain(Kind::ALL.iter().map(|k| k.icon()))
            .map(|i| ItemStack::new(i, 1))
            .collect()
    }

    fn legacy_templates(&mut self) -> Vec<Template> {
        let count = self.recipes().crafting.len();
        if self.pad.legacy.build.templates.as_ref().map(|t| t.0) != Some(count) {
            let all = legacy_build::templates(self.recipes());
            self.pad.legacy.build.templates = Some((count, all));
        }
        self.pad
            .legacy
            .build
            .templates
            .as_ref()
            .map_or_else(Vec::new, |t| t.1.clone())
    }

    /// The staged cells of a template: kept while the inventory still holds them, else staged
    /// afresh from it.
    fn legacy_staged(&mut self, template: &Template) -> Vec<Option<ItemStack>> {
        let inventory = &self.inventory[..36];
        let kept = self.pad.legacy.build.staged.get(&template.recipe);
        // While a run clicks, its stacks pass through the hand: the staging holds still.
        if self.pad.legacy.build.run.is_some()
            && let Some(kept) = kept
        {
            return kept.clone();
        }
        match kept {
            Some(kept) => legacy_build::restage(&self.loop_context, template, kept, inventory),
            None => legacy_build::stage(&self.loop_context, template, inventory),
        }
    }

    /// What the staging makes in this grid, if anything.
    fn legacy_makes(&self, template: &Template, staged: &[Option<ItemStack>]) -> Option<ItemStack> {
        let grid = self.craft_size;
        legacy_build::layout(template, staged, grid)
            .and_then(|cells| self.recipes().craft(&self.loop_context, &cells, grid))
            .filter(|_| legacy_build::available(staged, &self.inventory[..36]))
    }

    /// The selected builder: its templates, its row entries, the selected entry and variant, and
    /// that template's staged cells.
    fn legacy_build_current(&mut self) -> Option<Now> {
        let kind = self.legacy_builder()?;
        let ours: Vec<Template> = self
            .legacy_templates()
            .into_iter()
            .filter(|t| t.kind == kind)
            .collect();
        let groups = legacy_build::groups(&ours);
        let (entry, picked) = self
            .pad
            .legacy
            .build
            .selected
            .get(&kind)
            .copied()
            .unwrap_or_default();
        let last = groups.len().checked_sub(1)?;
        let makes_any = |game: &mut Self, members: &[usize]| {
            members.iter().any(|&m| {
                let t = ours[m].clone();
                let staged = game.legacy_staged(&t);
                game.legacy_makes(&t, &staged).is_some()
            })
        };
        let entry = match entry {
            Some(e) => e.min(last),
            None => (0..=last)
                .find(|&e| makes_any(self, &groups[e]))
                .unwrap_or(0),
        };
        let members = &groups[entry];
        let variant = match picked {
            Some(v) => v.min(members.len() - 1),
            None => (0..members.len())
                .find(|&v| {
                    let t = ours[members[v]].clone();
                    let staged = self.legacy_staged(&t);
                    self.legacy_makes(&t, &staged).is_some()
                })
                .unwrap_or(0),
        };
        let template = ours[members[variant]].clone();
        let staged = self.legacy_staged(&template);
        self.pad
            .legacy
            .build
            .staged
            .insert(template.recipe.clone(), staged.clone());
        Some(Now {
            ours,
            groups,
            entry,
            variant,
            template,
            staged,
        })
    }

    /// A step along a builder's row (left and right) or through its entry's variants (up and
    /// down): whether it went anywhere.
    pub(in crate::game) fn legacy_build_step(&mut self, button: Button) -> bool {
        let Some(kind) = self.legacy_builder() else {
            return false;
        };
        let Some(now) = self.legacy_build_current() else {
            return false;
        };
        let n = now.groups.len().max(1);
        let m = now.groups[now.entry].len().max(1);
        let to = match button {
            Button::DPadLeft => (Some((now.entry + n - 1) % n), None),
            Button::DPadRight => (Some((now.entry + 1) % n), None),
            Button::DPadUp if m > 1 => (Some(now.entry), Some((now.variant + m - 1) % m)),
            Button::DPadDown if m > 1 => (Some(now.entry), Some((now.variant + 1) % m)),
            _ => return false,
        };
        self.pad.legacy.build.selected.insert(kind, to);
        true
    }

    /// A press in a builder: whether it was the builder's. On the row (or the grid) A crafts one
    /// and Y as many as the items allow; in the grid A and X step the focused cell through what
    /// fits it.
    pub(in crate::game) fn legacy_build_button(&mut self, button: Button) -> bool {
        if self.legacy_builder().is_none() {
            return false;
        }
        let focus = self.pad.legacy.focus;
        match (focus, button) {
            (Area::Grid, Button::South) => self.legacy_build_cycle(true),
            (Area::Grid, Button::West) => self.legacy_build_cycle(false),
            (Area::Row, Button::South) => self.legacy_build_start(1),
            (Area::Row | Area::Grid, Button::North) => self.legacy_build_start(MOST),
            // X on the row: no craftable filter here.
            (Area::Row, Button::West) => {}
            _ => return false,
        }
        true
    }

    /// A mouse click in a builder: its own selection and craft, as its buttons make them (the
    /// recipe book's selection isn't the builder's). Whether it was the builder's.
    pub(in crate::game) fn legacy_build_click(&mut self, hit: Hit, right: bool) -> bool {
        let Some(kind) = self.legacy_builder() else {
            return false;
        };
        match hit {
            // A click on the selected entry crafts it, as A does; on another, selects it.
            Hit::Row(i) => {
                self.pad.legacy.focus = Area::Row;
                let entry = self.legacy_build_current().map(|now| now.entry);
                if entry == Some(i) && !right {
                    self.legacy_build_start(1);
                } else {
                    self.pad.legacy.build.selected.insert(kind, (Some(i), None));
                }
            }
            Hit::Variant(i) => {
                if let Some(now) = self.legacy_build_current() {
                    self.pad
                        .legacy
                        .build
                        .selected
                        .insert(kind, (Some(now.entry), Some(i)));
                }
            }
            // The result: one craft, or (right click) as many as the items allow, as A and Y.
            Hit::Result => self.legacy_build_start(if right { MOST } else { 1 }),
            // A grid cell: its item stepped on (right click: back), as A and X there.
            Hit::Cell(i) => {
                self.pad.legacy.focus = Area::Grid;
                self.pad.legacy.cursor = i;
                self.legacy_build_cycle(!right);
            }
            _ => return false,
        }
        true
    }

    /// The focused grid cell's staged item, stepped to the next (or previous) the inventory has
    /// that fits it; an optional cell steps through empty too.
    fn legacy_build_cycle(&mut self, next: bool) {
        let Some(Now {
            template,
            mut staged,
            ..
        }) = self.legacy_build_current()
        else {
            return;
        };
        let template = &template;
        let grid = self.craft_size;
        let cell = self.pad.legacy.cursor;
        let Some(i) = (0..template.cells.len()).find(|&i| template.cell_in(i, grid) == Some(cell))
        else {
            // A cell the recipe doesn't use.
            self.pad_rumble(gamepad::Rumble::RadialNothing);
            return;
        };
        let role = &template.cells[i].1;
        let inventory = &self.inventory[..36];
        let mut choices: Vec<Option<ItemStack>> =
            legacy_build::candidates(&self.loop_context, role, inventory)
                .into_iter()
                .map(Some)
                .collect();
        if role.optional {
            choices.insert(0, None);
        }
        if choices.is_empty() {
            self.pad_rumble(gamepad::Rumble::RadialNothing);
            return;
        }
        let same = |a: &Option<ItemStack>, b: &Option<ItemStack>| match (a, b) {
            (Some(a), Some(b)) => a.same_item_same_components(b),
            (None, None) => true,
            _ => false,
        };
        let at = choices.iter().position(|c| same(c, &staged[i]));
        let n = choices.len();
        let to = match (at, next) {
            (Some(at), true) => (at + 1) % n,
            (Some(at), false) => (at + n - 1) % n,
            (None, _) => 0,
        };
        staged[i] = choices[to].clone();
        self.pad
            .legacy
            .build
            .staged
            .insert(template.recipe.clone(), staged);
    }

    /// Starts a craft of the selected template: `count` of them at most. Refused (a bump) with
    /// something in hand, where crafting isn't allowed, or when the staging makes nothing here.
    fn legacy_build_start(&mut self, count: u32) {
        let Some(Now {
            template, staged, ..
        }) = self.legacy_build_current()
        else {
            return;
        };
        let template = &template;
        let makes = self.legacy_makes(template, &staged).is_some();
        let busy = self.pad.legacy.build.run.is_some();
        if busy || !makes || self.carried.is_some() || !self.direct_craft_allowed() {
            self.pad_rumble(gamepad::Rumble::RadialNothing);
            return;
        }
        let Some(steps) = self.legacy_build_steps(template, &staged) else {
            self.pad_rumble(gamepad::Rumble::RadialNothing);
            return;
        };
        let cells = self.craft_size * self.craft_size;
        let menu = self.remote_menu().0;
        self.pad.legacy.build.run = Some(Run {
            menu,
            recipe: template.recipe.clone(),
            steps,
            next_at: Instant::now(),
            result: RESULT,
            inputs: (GRID..GRID + cells).collect(),
            more: count - 1,
        });
    }

    /// One craft's steps: what's in the grid goes back first, then each staged item in.
    fn legacy_build_steps(
        &self,
        template: &Template,
        staged: &[Option<ItemStack>],
    ) -> Option<VecDeque<Step>> {
        let grid = self.craft_size;
        let mut steps = VecDeque::from([Step::Clear, Step::Empty]);
        for (i, stack) in staged.iter().enumerate() {
            if let Some(stack) = stack {
                let cell = GRID + template.cell_in(i, grid)?;
                steps.push_back(Step::Put {
                    stack: stack.clone(),
                    cell,
                });
            }
        }
        steps.extend([Step::Result { until: None }, Step::Take, Step::Clear]);
        Some(steps)
    }

    /// Once a frame while the screen shows: the running craft's next click, when its gap is up.
    pub(in crate::game) fn legacy_build_poll(&mut self) {
        let Some(mut run) = self.pad.legacy.build.run.take() else {
            return;
        };
        // Its menu went (closed, another opened): nothing more goes out on any other.
        if !self.legacy_showing() || self.remote_menu().0 != run.menu {
            return;
        }
        let now = Instant::now();
        if now < run.next_at {
            self.pad.legacy.build.run = Some(run);
            return;
        }
        let gap = if self.integrated.is_some() {
            Duration::ZERO
        } else {
            CLICK_GAP
        };
        let Some(step) = run.steps.pop_front() else {
            // One craft done: another, while asked for and the staging still holds.
            // Another craft only when the last result went: a full inventory keeps it there.
            if run.more > 0
                && self.slot_view(run.result).is_none()
                && let Some(current) = self.legacy_build_current()
                && current.template.recipe == run.recipe
                && self
                    .legacy_makes(&current.template, &current.staged)
                    .is_some()
                && let Some(steps) = self.legacy_build_steps(&current.template, &current.staged)
            {
                run.steps = steps;
                run.more -= 1;
                run.next_at = now + gap;
                self.pad.legacy.build.run = Some(run);
            }
            return;
        };
        match step {
            Step::Empty => {
                if run.inputs.iter().any(|&s| self.slot_view(s).is_some()) {
                    run.steps.clear();
                    run.more = 0;
                }
            }
            Step::Put { stack, cell } => {
                // Something already in hand, or another item in the cell: a click would swap
                // them. What went in comes back out, and that's the end.
                let other = self
                    .slot_view(cell)
                    .is_some_and(|s| !s.same_item_same_components(&stack));
                if self.carried.is_some() || other {
                    run.steps = VecDeque::from([Step::Clear]);
                    run.more = 0;
                    self.pad.legacy.build.run = Some(run);
                    return;
                }
                let from = (0..36).find(|&s| {
                    self.inventory[s]
                        .as_ref()
                        .is_some_and(|i| i.same_item_same_components(&stack))
                });
                let Some(from) = from else {
                    // It's gone: what went in comes back out, and that's the end.
                    run.steps = VecDeque::from([Step::Clear]);
                    run.more = 0;
                    self.pad.legacy.build.run = Some(run);
                    return;
                };
                for click in [
                    Step::Click {
                        slot: from,
                        right: false,
                        shift: false,
                    },
                    Step::Click {
                        slot: cell,
                        right: true,
                        shift: false,
                    },
                    Step::Click {
                        slot: from,
                        right: false,
                        shift: false,
                    },
                ]
                .into_iter()
                .rev()
                {
                    run.steps.push_front(click);
                }
            }
            Step::Click { slot, right, shift } => {
                self.legacy_slot_click(slot, right, shift);
                run.next_at = now + gap;
            }
            Step::Result { until } => {
                let until = until.unwrap_or(now + RESULT_WAIT);
                if self.slot_view(run.result).is_none() {
                    if now < until {
                        run.steps.push_front(Step::Result { until: Some(until) });
                    } else {
                        // No result: the grid empties and the craft ends.
                        run.steps = VecDeque::from([Step::Clear]);
                        run.more = 0;
                    }
                }
            }
            Step::Take => {
                if let Some(result) = self.slot_view(run.result) {
                    if self.legacy_room_for(&result) >= result.count {
                        self.legacy_slot_click(run.result, false, true);
                        run.next_at = now + gap;
                    } else {
                        // Not all of it fits: the menu would drop the rest (a crafting result's
                        // quick move throws what's left). What went in comes back instead.
                        run.steps = VecDeque::from([Step::Clear]);
                        run.more = 0;
                    }
                }
            }
            Step::Clear => {
                // A stack in hand (a Put cut short between its clicks) goes down first: on its
                // own kind where there's room, else an empty slot. Nowhere: it stays, and so do
                // the inputs (a click on one would swap them).
                if let Some(carried) = self.carried.clone() {
                    match self.legacy_stow_slot(&carried) {
                        Some(slot) => {
                            self.legacy_slot_click(slot, false, false);
                            run.next_at = now + gap;
                            // Again only if the click put some down: one that didn't would
                            // repeat for ever, the run never ending.
                            let less = self.carried.as_ref().is_none_or(|now| {
                                !now.same_item_same_components(&carried)
                                    || now.count < carried.count
                            });
                            if less {
                                run.steps.push_front(Step::Clear);
                            } else {
                                run.steps.clear();
                            }
                        }
                        None => run.steps.clear(),
                    }
                    self.pad.legacy.build.run = Some(run);
                    return;
                }
                let inputs = run.inputs.clone();
                if let Some(slot) = inputs.into_iter().find(|&s| self.slot_view(s).is_some()) {
                    self.legacy_slot_click(slot, false, true);
                    run.next_at = now + gap;
                    // Until the inputs are empty (a full inventory leaves one: the next look stops).
                    if self.slot_view(slot).is_none() {
                        run.steps.push_front(Step::Clear);
                    }
                }
            }
        }
        self.pad.legacy.build.run = Some(run);
    }

    /// Whether a builder's click run is under way: the
    /// player's own slot clicks, flicks and mode changes wait for it (B ends it).
    pub(in crate::game) fn legacy_busy(&self) -> bool {
        self.pad.legacy.build.run.is_some()
    }

    /// Ends whatever run is under way (B while one runs); what it put in comes back.
    /// How many of a stack the player's 36 slots can take.
    fn legacy_room_for(&self, stack: &ItemStack) -> u32 {
        let max = self.stack_max_size(stack);
        self.inventory[..36]
            .iter()
            .map(|slot| match slot {
                None => max,
                Some(s) if s.same_item_same_components(stack) => max.saturating_sub(s.count),
                Some(_) => 0,
            })
            .sum()
    }

    /// Where a stack in hand goes down: a slot of its kind with room, else an empty one.
    fn legacy_stow_slot(&self, stack: &ItemStack) -> Option<usize> {
        let max = self.stack_max_size(stack);
        let slots = &self.inventory[..36];
        (0..36)
            .find(|&s| {
                slots[s]
                    .as_ref()
                    .is_some_and(|i| i.same_item_same_components(stack) && i.count < max)
            })
            .or_else(|| (0..36).find(|&s| slots[s].is_none()))
    }

    pub(in crate::game) fn legacy_cancel_run(&mut self) {
        if let Some(run) = self.pad.legacy.build.run.as_mut() {
            run.steps = VecDeque::from([Step::Clear]);
            run.more = 0;
        }
        self.direct_craft_stop();
    }

    /// Whether a builder's craft is running.
    pub(in crate::game) fn legacy_build_running(&self) -> bool {
        self.pad.legacy.build.run.is_some()
    }

    /// A builder's view: its templates along the row, the staged grid, the game's result.
    pub(in crate::game) fn legacy_build_view(&mut self, view: &mut View, kind: Kind) {
        let tr = |key: &str| horizon_client::lang::get(key);
        view.tabs.clear();
        view.title = tr(kind.title_key());
        let Some(now) = self.legacy_build_current() else {
            view.empty = tr("horizon.legacy_craft.empty");
            return;
        };
        let grid = self.craft_size;
        let mut slot_of = |t: &Template| {
            let staged = self.legacy_staged(t);
            let made = self.legacy_makes(t, &staged);
            RecipeSlot {
                craftable: made.is_some(),
                stack: made.unwrap_or_else(|| t.icon.clone()),
                fresh: false,
            }
        };
        // Each entry shows a variant that can be made, if one can; the selected one its own.
        let mut row = Vec::new();
        for (e, members) in now.groups.iter().enumerate() {
            let shown = if e == now.entry {
                slot_of(&now.ours[members[now.variant]])
            } else {
                let slots: Vec<RecipeSlot> =
                    members.iter().map(|&m| slot_of(&now.ours[m])).collect();
                let first = slots.iter().position(|s| s.craftable).unwrap_or(0);
                slots[first].clone()
            };
            row.push(shown);
        }
        view.row = row;
        view.selected = now.entry;
        let members = &now.groups[now.entry];
        if members.len() > 1 {
            view.variants = members.iter().map(|&m| slot_of(&now.ours[m])).collect();
            view.variant = now.variant;
        }
        let template = &now.template;
        let staged = now.staged.clone();
        let made = self.legacy_makes(template, &staged);
        // The staged cells where they go in this grid; the template's other cells show what
        // they'd take, faded as missing.
        let mut cells: Vec<Option<Cell>> = vec![None; grid * grid];
        for (i, (_, role)) in template.cells.iter().enumerate() {
            let Some(at) = template.cell_in(i, grid) else {
                continue;
            };
            cells[at] = match &staged[i] {
                Some(stack) => Some(Cell {
                    stack: stack.clone(),
                    missing: false,
                }),
                None if !role.optional => role.accepts.order.first().map(|item| Cell {
                    stack: ItemStack::new(item, 1),
                    missing: true,
                }),
                None => None,
            };
        }
        view.cells = cells;
        view.name = horizon_client::stations::hover_name(
            &self.pack.names,
            made.as_ref().unwrap_or(&template.icon),
        );
        view.result = Some(RecipeSlot {
            stack: made.clone().unwrap_or_else(|| template.icon.clone()),
            craftable: made.is_some(),
            fresh: false,
        });
        view.note = Some(if !template.fits(grid) {
            (tr("horizon.legacy_craft.needs_table"), screen::MISSING)
        } else if self.carried.is_some() {
            (tr("horizon.legacy_craft.holding"), screen::MISSING)
        } else if made.is_none() {
            (tr("horizon.legacy_craft.not_yet"), screen::MISSING)
        } else if self.legacy_build_running() {
            (tr("horizon.legacy_craft.crafting"), screen::HAVE)
        } else {
            (tr("horizon.legacy_craft.ready"), screen::HAVE)
        });
    }
}
