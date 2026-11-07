//! A mod's recipes shown in its item panel's place (`native_panel_recipes`,
//! or `browser.set-recipes` of a WebAssembly mod, docs/mods.md), laid out by the game with its own slots, arrows and
//! buttons, paged to the panel's height, under a title and a back button.
//! The mod hears clicks on their slots, their action buttons, the back
//! button and page turns.
use super::{
    Game,
    native_hooks::{
        NativePanelEvent as BrowserEvent, NativeRecipeLayout as BrowserLayout,
        NativeRecipeView as BrowserRecipe, NativeStack as BrowserStack,
    },
    native_panel::{Layout, Owner, browser_stack, inside},
};
use horizon_client::{
    hud::{HudBatch, rgba},
    items::ItemStack,
};
use std::time::Instant;

const SLOT: f32 = 18.0;
/// The title row's height, and the gap under each recipe.
const TITLE: f32 = 18.0;
/// A tab's step: an 18 px slot and a gap.
const TAB: f32 = 20.0;
const GAP: f32 = 6.0;

/// Recipes a page may show, slots in one, stacks a slot cycles, and tabs.
const RECIPES: usize = 256;
const RECIPE_SLOTS: usize = 9;
const SLOT_STACKS: usize = 64;
const TABS: usize = 16;

// The default category order from JEI 31.17.0.108, for the stations Recipes exposes.
const JEI_STATIONS: [&str; 8] = [
    "minecraft:crafting_table",
    "minecraft:blast_furnace",
    "minecraft:brewing_stand",
    "minecraft:campfire",
    "minecraft:furnace",
    "minecraft:smithing_table",
    "minecraft:smoker",
    "minecraft:stonecutter",
];

/// What the panel shows instead of its items: a title, tabs (`tab` the
/// selected one) and the recipes.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct RecipePage {
    pub(super) title: String,
    pub(super) tabs: Vec<BrowserStack>,
    pub(super) tab: u32,
    pub(super) recipes: Vec<BrowserRecipe>,
}
impl RecipePage {
    /// `native_panel_recipes`' page, cut as `set-recipes` cuts one; none
    /// without recipes (the items again).
    pub(super) fn new(
        title: &str,
        tabs: Vec<BrowserStack>,
        tab: u32,
        recipes: Vec<BrowserRecipe>,
    ) -> Option<Self> {
        let cycle = |mut list: Vec<BrowserStack>| {
            list.truncate(SLOT_STACKS);
            list
        };
        let recipes: Vec<BrowserRecipe> = recipes
            .into_iter()
            .take(RECIPES)
            .map(|r| BrowserRecipe {
                width: r.width.clamp(1, 3),
                height: r.height.clamp(1, 3),
                slots: r.slots.into_iter().take(RECIPE_SLOTS).map(cycle).collect(),
                results: cycle(r.results),
                ..r
            })
            .collect();
        (!recipes.is_empty()).then(|| Self {
            title: title.to_owned(),
            tabs: tabs.into_iter().take(TABS).collect(),
            tab,
            recipes,
        })
    }
}

/// One recipe as the game draws it.
pub(super) struct Shown {
    layout: BrowserLayout,
    width: usize,
    height: usize,
    /// Each slot's stacks (the result last), as the mod sent them and as
    /// stacks; a stack the game doesn't have is left out of the cycle.
    slots: Vec<Vec<(BrowserStack, ItemStack)>>,
    note: String,
    action: Option<bool>,
}

/// The recipes a panel shows, and its page.
pub(super) struct Pages {
    title: String,
    /// The tabs' icons and the selected one.
    tabs: Vec<(BrowserStack, Option<ItemStack>)>,
    tab: usize,
    tab_events: Vec<usize>,
    recipes: Vec<Shown>,
    page: usize,
    since: Instant,
}

impl Pages {
    pub(super) fn fits_recipe_tabs(&self) -> bool {
        self.tabs.len() <= 8
    }
    pub(super) fn new(
        items: &horizon_client::items::Items,
        shown: &RecipePage,
        old: Option<Pages>,
        official: bool,
    ) -> Self {
        let (title, list) = (&shown.title, &shown.recipes);
        let mut tab_events: Vec<_> = (0..shown.tabs.len()).collect();
        if official {
            tab_events.sort_by_key(|&i| {
                JEI_STATIONS
                    .iter()
                    .position(|&station| station == shown.tabs[i].item)
                    .unwrap_or(JEI_STATIONS.len())
            });
        }
        let tab = tab_events
            .iter()
            .position(|&i| i == shown.tab as usize)
            .unwrap_or(0);
        let kept = old.filter(|o| o.title == *title && o.tab == tab);
        let cycle = |stacks: &[BrowserStack]| -> Vec<(BrowserStack, ItemStack)> {
            stacks
                .iter()
                .filter_map(|s| Some((s.clone(), browser_stack(items, s)?)))
                .collect()
        };
        let recipes = list
            .iter()
            .map(|r| {
                let mut slots: Vec<_> = r.slots.iter().map(|s| cycle(s)).collect();
                slots.push(cycle(&r.results));
                Shown {
                    layout: r.layout,
                    width: r.width as usize,
                    height: r.height as usize,
                    slots,
                    note: r.note.clone(),
                    action: r.action,
                }
            })
            .collect();
        Self {
            title: title.clone(),
            tabs: tab_events
                .iter()
                .map(|&i| {
                    let t = &shown.tabs[i];
                    (t.clone(), browser_stack(items, t))
                })
                .collect(),
            tab,
            tab_events,
            recipes,
            page: kept.as_ref().map_or(0, |o| o.page),
            since: kept.map_or_else(Instant::now, |o| o.since),
        }
    }
}

impl Shown {
    /// The inputs' block: its size and each input slot's place in it.
    fn inputs(&self) -> ((f32, f32), Vec<(f32, f32)>) {
        let row = |n: usize| (0..n).map(|i| (i as f32 * SLOT, 0.0)).collect();
        match self.layout {
            BrowserLayout::Grid => {
                let (w, h) = (self.width.max(1), self.height.max(1));
                let at = (0..w * h)
                    .map(|i| ((i % w) as f32 * SLOT, (i / w) as f32 * SLOT))
                    .collect();
                ((w as f32 * SLOT, h as f32 * SLOT), at)
            }
            // The input over the flame.
            BrowserLayout::Cooking => ((SLOT, 2.0 * SLOT), vec![(0.0, 0.0)]),
            BrowserLayout::Single => ((SLOT, SLOT), row(1)),
            BrowserLayout::Smithing => ((3.0 * SLOT, SLOT), row(3)),
            BrowserLayout::Brewing => ((2.0 * SLOT, SLOT), row(2)),
        }
    }
    /// The height of its slots, arrow and button, `room` wide.
    fn block(&self, layout: &Layout) -> f32 {
        let h = self.inputs().0.1;
        let (_, _, action) = self.places(layout);
        let h = if layout.centered && self.layout == BrowserLayout::Grid {
            3.0 * SLOT
        } else {
            h
        };
        match self.action {
            Some(_) => h.max(action.1 + 12.0),
            None => h,
        }
    }
    fn height(&self, layout: &Layout) -> f32 {
        let note = if self.note.is_empty() { 0.0 } else { 11.0 };
        self.block(layout) + note + GAP
    }
    /// Every slot's place from the recipe's corner (the result last), the
    /// arrow's and the action button's: beside the result, or under it
    /// when the panel is `room` wide and that doesn't fit.
    fn places(&self, layout: &Layout) -> (Vec<(f32, f32)>, (f32, f32), (f32, f32)) {
        let ((mut w, mut h), mut at) = self.inputs();
        if layout.centered && self.layout == BrowserLayout::Grid {
            (w, h) = (3.0 * SLOT, 3.0 * SLOT);
        }
        // The mod's slots beyond the layout's are not shown.
        at.truncate(self.slots.len() - 1);
        let arrow = (w + 4.0, ((h - 16.0) / 2.0).floor());
        let result = (
            w + if layout.centered { 40.0 } else { 32.0 },
            ((h - SLOT) / 2.0).floor(),
        );
        let action = if layout.centered {
            (result.0 + SLOT + 8.0, result.1 + SLOT + 9.0)
        } else if result.0 + SLOT + 16.0 <= layout.width() {
            (result.0 + SLOT + 4.0, ((h - 12.0) / 2.0).floor())
        } else {
            (result.0 + 3.0, result.1 + SLOT + 2.0)
        };
        while at.len() < self.slots.len() - 1 {
            at.push((f32::NAN, f32::NAN));
        }
        at.push(result);
        (at, arrow, action)
    }
}

/// The tabs' places, in rows under the title row.
fn tab_places(pages: &Pages, layout: &Layout) -> Vec<(f32, f32)> {
    if layout.centered {
        return (0..pages.tabs.len())
            .map(|i| {
                (
                    layout.area[0] + 2.0 + i as f32 * 24.0,
                    layout.area[1] - 20.0,
                )
            })
            .collect();
    }
    let per_row = ((layout.width() / TAB).floor() as usize).max(1);
    (0..pages.tabs.len())
        .map(|i| {
            (
                layout.grid.0 + (i % per_row) as f32 * TAB,
                layout.grid.1 + TITLE + 3.0 + (i / per_row) as f32 * TAB,
            )
        })
        .collect()
}

fn tab_rect(layout: &Layout, x: f32, y: f32) -> [f32; 4] {
    if layout.centered {
        [x, y, 24.0, 22.0]
    } else {
        [x, y, SLOT, SLOT]
    }
}

fn category_arrows(layout: &Layout) -> [[f32; 4]; 2] {
    layout
        .recipe_arrows()
        .map(|[x, y, w, h]| [x, y - 15.0, w, h])
}
/// Where the recipes start: under the title row and the tabs.
fn recipes_top(pages: &Pages, layout: &Layout) -> f32 {
    if layout.centered {
        return layout.area[1] + 48.0;
    }
    let tabs = tab_places(pages, layout);
    tabs.last()
        .map_or(layout.grid.1 + TITLE, |&(_, y)| y + TAB + 3.0)
}

fn recipe_skin(
    batch: &mut HudBatch,
    atlas: &horizon_client::hud::HudAtlas,
    sprite: &str,
    rect: [f32; 4],
) {
    let [x, y, w, h] = rect;
    horizon_client::ui::nine_slice(batch, atlas, sprite, x, y, w, h, u32::MAX);
}

impl Game<'_> {
    /// The first recipe of each page, filling the panel from the tabs to
    /// the search box.
    fn recipe_pages(pages: &Pages, layout: &Layout) -> Vec<usize> {
        let room = layout.search[1] - 4.0 - recipes_top(pages, layout);
        let mut starts = vec![0];
        let mut used = 0.0;
        for (i, r) in pages.recipes.iter().enumerate() {
            let h = r.height(layout);
            if used > 0.0 && used + h > room {
                starts.push(i);
                used = 0.0;
            }
            used += h;
        }
        starts
    }
    /// The recipes on the open page, each with its corner.
    fn recipes_shown(&self, layout: &Layout) -> Vec<(usize, (f32, f32))> {
        let Some(pages) = &self.panel.recipes else {
            return Vec::new();
        };
        let starts = Self::recipe_pages(pages, layout);
        let page = pages.page.min(starts.len() - 1);
        let end = starts.get(page + 1).copied().unwrap_or(pages.recipes.len());
        let mut y = recipes_top(pages, layout);
        (starts[page]..end)
            .map(|i| {
                let at = (
                    if layout.centered {
                        layout.area[0] + 41.0
                    } else {
                        layout.grid.0
                    },
                    y,
                );
                y += pages.recipes[i].height(layout);
                (i, at)
            })
            .collect()
    }
    fn back_button(&self, layout: &Layout) -> ([f32; 4], String) {
        let label = self.translate("gui.back", "Back");
        if layout.centered {
            let [x, y, _, h] = layout.area;
            return ([x - 18.0, y + h - 20.0, 15.0, 15.0], label);
        }
        let w = HudBatch::text_width(&self.gui.atlas, &label) + 8.0;
        ([layout.grid.0, layout.grid.1, w, 14.0], label)
    }
    /// What a slot of a shown recipe holds now: its stacks cycle once a second.
    fn recipe_slot_now(&self, recipe: usize, slot: usize) -> Option<&(BrowserStack, ItemStack)> {
        let pages = self.panel.recipes.as_ref()?;
        let stacks = pages.recipes.get(recipe)?.slots.get(slot)?;
        let n = pages.since.elapsed().as_secs() as usize;
        stacks.get(n % stacks.len().max(1))
    }
    /// The tab under the pointer: its index and icon.
    pub(super) fn recipe_tab_hovered(
        &self,
        layout: &Layout,
    ) -> Option<(usize, BrowserStack, ItemStack)> {
        let pages = self.panel.recipes.as_ref()?;
        let places = tab_places(pages, layout);
        places.iter().enumerate().find_map(|(i, &(x, y))| {
            if !inside(self.cursor, tab_rect(layout, x, y)) {
                return None;
            }
            let (sent, stack) = pages.tabs.get(i)?;
            Some((i, sent.clone(), stack.clone()?))
        })
    }
    /// The recipe slot under the pointer: recipe, slot, and its stack now.
    pub(super) fn recipe_slot_hovered(
        &self,
        layout: &Layout,
    ) -> Option<(usize, usize, BrowserStack, ItemStack)> {
        let pages = self.panel.recipes.as_ref()?;
        self.recipes_shown(layout)
            .into_iter()
            .find_map(|(i, (x, y))| {
                let (at, _, _) = pages.recipes[i].places(layout);
                at.iter().enumerate().find_map(|(k, &(sx, sy))| {
                    if !inside(self.cursor, [x + sx, y + sy, SLOT, SLOT]) {
                        return None;
                    }
                    let (mod_stack, stack) = self.recipe_slot_now(i, k)?.clone();
                    Some((i, k, mod_stack, stack))
                })
            })
    }

    fn draw_recipes_button(&self, batch: &mut HudBatch, [x, y, w, h]: [f32; 4], enabled: bool) {
        let rect = [x, y, w, h];
        let sprite = match (enabled, inside(self.cursor, rect)) {
            (false, _) => "horizon/jei/button_disabled_v2",
            (true, false) => "horizon/jei/button_enabled_v2",
            (true, true) => "horizon/jei/button_highlight_v2",
        };
        recipe_skin(batch, &self.gui.atlas, sprite, rect);
    }

    fn draw_recipes_arrow(
        &self,
        batch: &mut HudBatch,
        rect: [f32; 4],
        forward: bool,
        enabled: bool,
    ) {
        self.draw_recipes_button(batch, rect, enabled);
        let [x, y, w, h] = rect;
        let sprite = if forward {
            "horizon/jei/icons/arrow_next_v2"
        } else {
            "horizon/jei/icons/arrow_previous_v2"
        };
        batch.sprite(
            &self.gui.atlas,
            sprite,
            x + ((w - 9.0) / 2.0).floor(),
            y + ((h - 7.0) / 2.0).floor(),
        );
    }

    pub(super) fn draw_browser_arrows(
        &self,
        batch: &mut HudBatch,
        bounds: [[f32; 4]; 2],
        labels: &[String; 2],
        enabled: [bool; 2],
        square: bool,
    ) {
        for (i, (rect, on)) in bounds.into_iter().zip(enabled).enumerate() {
            if square {
                self.draw_recipes_arrow(batch, rect, i == 1, on);
            } else if on {
                let name = ["page_backward", "page_forward"][i];
                let suffix = if inside(self.cursor, rect) {
                    "_highlighted"
                } else {
                    ""
                };
                batch.sprite(
                    &self.gui.atlas,
                    &format!("gui/sprites/recipe_book/{name}{suffix}"),
                    rect[0],
                    rect[1],
                );
            } else {
                continue;
            }
            horizon_client::widgets::record(&labels[i], rect, on);
        }
    }

    /// Draws the title row, the page's recipes and the page arrows.
    pub(super) fn draw_browser_recipes(&mut self, batch: &mut HudBatch, layout: &Layout) {
        let Some(pages) = &self.panel.recipes else {
            return;
        };
        let cursor = self.cursor;
        let starts = Self::recipe_pages(pages, layout);
        let page = pages.page.min(starts.len() - 1);
        let title = if layout.centered {
            self.recipe_category_title(pages)
        } else {
            pages.title.clone()
        };
        let pages_n = starts.len();
        let (back, label) = self.back_button(layout);
        let atlas = &self.gui.atlas;
        if layout.centered {
            let [x, y, w, h] = layout.area;
            recipe_skin(
                batch,
                atlas,
                "horizon/jei/gui_background_v2",
                [x, y + 1.0, w, h + 2.0],
            );
            self.draw_recipes_button(batch, back, true);
            batch.text(atlas, "<", back[0] + 5.0, back[1] + 3.0, u32::MAX, true);
            horizon_client::widgets::record(&label, back, true);
            for top in [y + 5.0, y + 20.0] {
                batch.fill(
                    atlas,
                    x + 6.0,
                    top,
                    w - 12.0,
                    13.0,
                    rgba(161, 161, 161, 255),
                );
            }
        } else {
            let hover = inside(cursor, back);
            horizon_client::widgets::declare_action(
                horizon_client::widgets::Action::Back,
                back,
                true,
            );
            let sprite = if hover {
                "gui/sprites/widget/button_highlighted"
            } else {
                "gui/sprites/widget/button"
            };
            horizon_client::ui::nine_slice(
                batch,
                atlas,
                sprite,
                back[0],
                back[1],
                back[2],
                back[3],
                u32::MAX,
            );
            batch.text(atlas, &label, back[0] + 4.0, back[1] + 3.0, u32::MAX, true);
            horizon_client::widgets::record(&label, back, true);
        }
        let room = layout.width() - if layout.centered { 36.0 } else { back[2] + 4.0 };
        // Cut to the panel with vanilla's ellipsis (CommonComponents.ELLIPSIS).
        let shown = if atlas.font.fit(&title, room) >= title.chars().count() {
            title
        } else {
            let fit = atlas.font.fit(&title, room - atlas.font.width("..."));
            let kept: String = title.chars().take(fit).collect();
            format!("{}...", kept.trim_end())
        };
        batch.text(
            atlas,
            &shown,
            if layout.centered {
                (layout.area[0] + (layout.area[2] - HudBatch::text_width(atlas, &shown)) / 2.0)
                    .floor()
                    + 1.0
            } else {
                back[0] + back[2] + 4.0
            },
            if layout.centered {
                layout.area[1] + 7.0
            } else {
                back[1] + 3.0
            },
            u32::MAX,
            true,
        );
        if layout.centered {
            let labels = [
                self.translate("horizon.recipes.previous_category", "Previous Category"),
                self.translate("horizon.recipes.next_category", "Next Category"),
            ];
            self.draw_browser_arrows(
                batch,
                category_arrows(layout),
                &labels,
                [pages.tab > 0, pages.tab + 1 < pages.tabs.len()],
                true,
            );
        }
        if pages_n > 1 || layout.centered {
            let labels = [
                self.translate("gui.recipebook.previous_page", "Previous Page"),
                self.translate("gui.recipebook.next_page", "Next Page"),
            ];
            let visible = [page > 0, page + 1 < pages_n];
            let atlas = &self.gui.atlas;
            self.draw_browser_arrows(
                batch,
                layout.recipe_arrows(),
                &labels,
                visible,
                layout.centered,
            );
            let text = format!("{}/{pages_n}", page + 1);
            let w = HudBatch::text_width(atlas, &text);
            let x = (layout.grid.0 + (layout.width() - w) / 2.0).floor();
            batch.text(
                atlas,
                &text,
                x,
                layout.arrows_y + if layout.centered { 2.0 } else { 5.0 },
                u32::MAX,
                true,
            );
        }
        self.draw_recipe_tabs(batch, layout);
        if layout.centered {
            self.draw_recipe_stations(batch, layout);
        }
        for (i, (x, y)) in self.recipes_shown(layout) {
            let Some(r) = self.panel.recipes.as_ref().map(|p| &p.recipes[i]) else {
                break;
            };
            let (at, arrow, action) = r.places(layout);
            let h = r.block(layout);
            let (cooking, note, button) =
                (r.layout == BrowserLayout::Cooking, r.note.clone(), r.action);
            let atlas = &self.gui.atlas;
            if layout.centered && r.layout == BrowserLayout::Grid {
                recipe_skin(
                    batch,
                    atlas,
                    "horizon/jei/single_recipe_background_v2",
                    [x - 4.0, y - 4.0, 122.0, 61.0],
                );
                for k in 0..9 {
                    batch.sprite(
                        atlas,
                        "horizon/jei/slot",
                        x + (k % 3) as f32 * SLOT,
                        y + (k / 3) as f32 * SLOT,
                    );
                }
            }
            for (slot, &(sx, sy)) in at.iter().enumerate().filter(|(_, p)| p.0.is_finite()) {
                if layout.centered && slot + 1 == at.len() {
                    let (x, y) = (x + sx - 4.0, y + sy - 4.0);
                    batch.sprite(atlas, "horizon/jei/output_slot", x, y);
                } else {
                    let sprite = if layout.centered {
                        "horizon/jei/slot"
                    } else {
                        "gui/sprites/container/slot"
                    };
                    batch.sprite(atlas, sprite, x + sx, y + sy);
                }
            }
            if cooking {
                batch.sprite(
                    atlas,
                    "gui/sprites/container/furnace/lit_progress",
                    x + 2.0,
                    y + SLOT + 2.0,
                );
            }
            if layout.centered && !cooking {
                let (x, y) = (x + arrow.0 + 4.0, y + arrow.1 + 8.0);
                let gray = rgba(135, 135, 135, 255);
                batch.fill(atlas, x, y - 1.5, 14.0, 3.0, gray);
                for i in 0..8 {
                    let n = i as f32;
                    batch.fill(atlas, x + 14.0 + n, y - 7.0 + n, 1.0, 15.0 - 2.0 * n, gray);
                }
            } else {
                batch.sprite(
                    atlas,
                    "gui/sprites/container/furnace/burn_progress",
                    x + arrow.0,
                    y + arrow.1,
                );
            }
            if let Some(enabled) = button {
                let rect = [x + action.0, y + action.1, 12.0, 12.0];
                let sprite = match (enabled, inside(cursor, rect)) {
                    (false, _) => "gui/sprites/widget/button_disabled",
                    (true, true) => "gui/sprites/widget/button_highlighted",
                    (true, false) => "gui/sprites/widget/button",
                };
                if layout.centered {
                    self.draw_recipes_button(batch, rect, enabled);
                    batch.sprite_scaled(
                        atlas,
                        "horizon/jei/icons/recipe_transfer",
                        rect[0] + 2.0,
                        rect[1] + 2.0,
                        7.0,
                        7.0,
                        u32::MAX,
                    );
                } else {
                    horizon_client::ui::nine_slice(
                        batch,
                        atlas,
                        sprite,
                        rect[0],
                        rect[1],
                        12.0,
                        12.0,
                        u32::MAX,
                    );
                    let color = if enabled {
                        u32::MAX
                    } else {
                        rgba(160, 160, 160, 255)
                    };
                    batch.text(atlas, "+", rect[0] + 3.0, rect[1] + 2.0, color, true);
                }
                horizon_client::widgets::record(&format!("+ {i}"), rect, enabled);
            }
            if !note.is_empty() {
                batch.text(atlas, &note, x, y + h + 2.0, rgba(224, 224, 224, 255), true);
            }
            for (k, &(sx, sy)) in at.iter().enumerate().filter(|(_, p)| p.0.is_finite()) {
                let Some((_, stack)) = self.recipe_slot_now(i, k).cloned() else {
                    continue;
                };
                self.draw_item(batch, &stack, x + sx + 1.0, y + sy + 1.0);
                if inside(cursor, [x + sx, y + sy, SLOT, SLOT]) {
                    batch.fill(
                        &self.gui.atlas,
                        x + sx + 1.0,
                        y + sy + 1.0,
                        16.0,
                        16.0,
                        rgba(255, 255, 255, 128),
                    );
                }
                let name = self.pack.names.display_item(&stack.item);
                horizon_client::widgets::record_as(
                    &format!("{name} {i}:{k}"),
                    [x + sx + 1.0, y + sy + 1.0, 16.0, 16.0],
                    true,
                    horizon_client::widgets::Kind::Button,
                );
            }
        }
    }

    fn recipe_category_title(&self, pages: &Pages) -> String {
        let station = pages
            .tabs
            .get(pages.tab)
            .map(|(stack, _)| stack.item.as_str());
        let (key, fallback) = match station {
            Some("minecraft:crafting_table") => ("container.crafting", "Crafting"),
            Some("minecraft:furnace") => ("horizon.recipes.category.smelting", "Smelting"),
            Some("minecraft:smoker") => ("horizon.recipes.category.smoking", "Smoking"),
            Some("minecraft:blast_furnace") => ("horizon.recipes.category.blasting", "Blasting"),
            Some("minecraft:campfire") => ("horizon.recipes.category.campfire", "Campfire Cooking"),
            Some("minecraft:stonecutter") => {
                ("horizon.recipes.category.stonecutting", "Stonecutting")
            }
            Some("minecraft:smithing_table") => ("horizon.recipes.category.smithing", "Smithing"),
            Some("minecraft:brewing_stand") => ("horizon.recipes.category.brewing", "Brewing"),
            _ => return pages.title.clone(),
        };
        self.translate(key, fallback)
    }

    /// The tabs, on slots; the selected one framed.
    fn draw_recipe_tabs(&mut self, batch: &mut HudBatch, layout: &Layout) {
        let Some(pages) = &self.panel.recipes else {
            return;
        };
        let places = tab_places(pages, layout);
        let tabs: Vec<Option<ItemStack>> = pages.tabs.iter().map(|t| t.1.clone()).collect();
        let selected = pages.tab;
        for (i, (&(x, y), stack)) in places.iter().zip(tabs).enumerate() {
            let atlas = &self.gui.atlas;
            let rect = tab_rect(layout, x, y);
            let hovered = inside(self.cursor, rect);
            if layout.centered {
                let sprite = if i == selected {
                    "horizon/jei/tab_selected"
                } else {
                    "horizon/jei/tab_unselected"
                };
                batch.sprite(atlas, sprite, x, y);
            } else {
                batch.sprite(atlas, "gui/sprites/container/slot", x, y);
            }
            let Some(stack) = stack else {
                continue;
            };
            let inset = if layout.centered { 4.0 } else { 1.0 };
            self.draw_item(batch, &stack, x + inset, y + inset);
            // The selected tab: a white frame around its slot.
            if i == selected && !layout.centered {
                let white = rgba(255, 255, 255, 255);
                let atlas = &self.gui.atlas;
                batch.fill(atlas, x - 1.0, y - 1.0, SLOT + 2.0, 1.0, white);
                batch.fill(atlas, x - 1.0, y + SLOT, SLOT + 2.0, 1.0, white);
                batch.fill(atlas, x - 1.0, y, 1.0, SLOT, white);
                batch.fill(atlas, x + SLOT, y, 1.0, SLOT, white);
            }
            if hovered && !layout.centered {
                batch.fill(
                    &self.gui.atlas,
                    x + 1.0,
                    y + 1.0,
                    16.0,
                    16.0,
                    rgba(255, 255, 255, 128),
                );
            }
            let name = self.pack.names.display_item(&stack.item);
            horizon_client::widgets::record_as(
                &format!("{name} tab"),
                if layout.centered {
                    rect
                } else {
                    [x + 1.0, y + 1.0, 16.0, 16.0]
                },
                true,
                horizon_client::widgets::Kind::Tab(i == selected),
            );
        }
    }

    fn draw_recipe_stations(&mut self, batch: &mut HudBatch, layout: &Layout) {
        let Some(pages) = &self.panel.recipes else {
            return;
        };
        let Some((_, Some(station))) = pages.tabs.get(pages.tab) else {
            return;
        };
        let mut stations = vec![station.clone()];
        if station.item.as_ref() == "minecraft:crafting_table" {
            stations.push(ItemStack::new("minecraft:crafter", 1));
        }
        let [left, y, _, _] = layout.area;
        let (x, y) = (left - 22.0, y + 1.0);
        recipe_skin(
            batch,
            &self.gui.atlas,
            "horizon/jei/catalyst_tab_v2",
            [x, y, 28.0, 10.0 + stations.len() as f32 * 18.0],
        );
        for (i, station) in stations.iter().enumerate() {
            let y = y + 5.0 + i as f32 * 18.0;
            batch.sprite(
                &self.gui.atlas,
                "horizon/jei/recipe_catalyst_slot_background_v2",
                x + 5.0,
                y,
            );
            self.draw_item(batch, station, x + 6.0, y + 1.0);
        }
    }

    /// A press on the recipes: true when it hit one of their parts.
    pub(super) fn browser_recipes_press(
        &mut self,
        layout: &Layout,
        owner: &Owner,
        right: bool,
    ) -> bool {
        let Some(pages) = &self.panel.recipes else {
            return false;
        };
        let cursor = self.cursor;
        let starts = Self::recipe_pages(pages, layout);
        let page = pages.page.min(starts.len() - 1);
        if layout.centered {
            let [previous, next] = category_arrows(layout);
            let category = if pages.tab > 0 && inside(cursor, previous) {
                Some(pages.tab - 1)
            } else if pages.tab + 1 < pages.tabs.len() && inside(cursor, next) {
                Some(pages.tab + 1)
            } else {
                None
            };
            if let Some(category) = category {
                self.audio
                    .play("minecraft:ui.button.click", None, 0.25, 1.0);
                let index = pages.tab_events[category];
                self.panel_event(owner, BrowserEvent::Tab(index as u32));
                return true;
            }
        }
        let [back_arrow, forward] = layout.recipe_arrows();
        let turn = if page > 0 && inside(cursor, back_arrow) {
            Some(page - 1)
        } else if page + 1 < starts.len() && inside(cursor, forward) {
            Some(page + 1)
        } else {
            None
        };
        if let Some(to) = turn {
            if let Some(p) = self.panel.recipes.as_mut() {
                p.page = to;
            }
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            self.panel_event(owner, BrowserEvent::Page(starts[to] as u32));
            return true;
        }
        if let Some((tab, _, _)) = self.recipe_tab_hovered(layout) {
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            self.panel_event(owner, BrowserEvent::Tab(pages.tab_events[tab] as u32));
            return true;
        }
        if inside(cursor, self.back_button(layout).0) {
            self.audio
                .play("minecraft:ui.button.click", None, 0.25, 1.0);
            self.panel_event(owner, BrowserEvent::Back);
            return true;
        }
        if let Some((recipe, slot, stack, _)) = self.recipe_slot_hovered(layout) {
            let event = BrowserEvent::SlotClicked {
                recipe: recipe as u32,
                slot: slot as u32,
                stack,
                right,
            };
            self.panel_event(owner, event);
            return true;
        }
        let action = self.recipes_shown(layout).into_iter().find(|&(i, (x, y))| {
            let r = &pages.recipes[i];
            let (_, _, (ax, ay)) = r.places(layout);
            r.action.is_some() && inside(cursor, [x + ax, y + ay, 12.0, 12.0])
        });
        if let Some((i, _)) = action {
            if pages.recipes[i].action == Some(true) {
                self.audio
                    .play("minecraft:ui.button.click", None, 0.25, 1.0);
                self.panel_event(owner, BrowserEvent::Action(i as u32));
            }
            return true;
        }
        false
    }
}
