//! The controller crafting screen's builders (Horizon's own; `legacy_craft.rs`): the crafting
//! recipes vanilla's recipe book can't show, the ones built from what's put in the grid
//! (`CustomRecipe`s and the rest of `Special`): fireworks, dyed armour, banners and shields,
//! decorated pots, map and book copies, tipped arrows.
//!
//! Each special recipe the game knows becomes a template: the cells it fills and what each cell
//! takes (its `Ingredient`), required or not. The screen stages items from the player's inventory
//! in those cells, asks the recipes what they make (`Recipes::craft`, the game's own matcher, so
//! the preview is the real result, colours and flight included), and crafts by putting the items
//! in the grid with vanilla's clicks. A mod's recipe of one of these kinds gets a template too.
use crate::{
    crafting::{Ingredient, Recipe, Recipes, Special},
    items::ItemStack,
};
use horizon_world::native_load_context::LoopContext;

/// The builders: the mode tabs after Crafting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Fireworks,
    Dyeing,
    Banners,
    Pottery,
    Copying,
    Arrows,
}

impl Kind {
    pub const ALL: [Kind; 6] = [
        Kind::Fireworks,
        Kind::Dyeing,
        Kind::Banners,
        Kind::Pottery,
        Kind::Copying,
        Kind::Arrows,
    ];
    /// The item on its tab.
    pub fn icon(self) -> &'static str {
        match self {
            Kind::Fireworks => "minecraft:firework_rocket",
            Kind::Dyeing => "minecraft:red_dye",
            Kind::Banners => "minecraft:shield",
            Kind::Pottery => "minecraft:decorated_pot",
            Kind::Copying => "minecraft:filled_map",
            Kind::Arrows => "minecraft:tipped_arrow",
        }
    }
    /// Its title's language key.
    pub fn title_key(self) -> &'static str {
        match self {
            Kind::Fireworks => "horizon.legacy_craft.builder.fireworks",
            Kind::Dyeing => "horizon.legacy_craft.builder.dyeing",
            Kind::Banners => "horizon.legacy_craft.builder.banners",
            Kind::Pottery => "horizon.legacy_craft.builder.pottery",
            Kind::Copying => "horizon.legacy_craft.builder.copying",
            Kind::Arrows => "horizon.legacy_craft.builder.arrows",
        }
    }
}

/// What a cell takes.
#[derive(Clone, Debug)]
pub struct Role {
    pub accepts: Ingredient,
    /// The recipe makes something with the cell empty.
    pub optional: bool,
    /// Whether the cell's banner or shield must carry patterns (`Some(true)`, a copy's source:
    /// at most `banner::MAX_LAYERS` of them, as `BannerDuplicateRecipe` takes), must be blank
    /// (`Some(false)`, a copy's target, a shield to decorate), or either.
    pub patterned: Option<bool>,
}

/// One special recipe as the builder stages it.
#[derive(Clone, Debug)]
pub struct Template {
    /// The recipe's id.
    pub recipe: String,
    pub kind: Kind,
    /// What its row slot shows.
    pub icon: ItemStack,
    /// Its cells: a position in a 3x3 grid (row by row) and what goes there. A shapeless
    /// recipe's come first to last and fill any grid in that order.
    pub cells: Vec<(usize, Role)>,
    /// The positions matter (a pot's four sides, a ring round a map): only a 3x3 grid has them.
    pub shaped: bool,
    /// What the row groups it with: the recipes of one family share an entry, their variants
    /// (each colour of dyeing a shulker box, each piece of armour a dye colours).
    pub family: String,
}

impl Template {
    /// Where its `i`th cell goes in a grid `grid` wide: none when the grid has no room for it.
    pub fn cell_in(&self, i: usize, grid: usize) -> Option<usize> {
        let (at, _) = self.cells.get(i)?;
        if self.shaped {
            (grid == 3).then_some(*at)
        } else {
            (i < grid * grid).then_some(i)
        }
    }

    /// The grid can hold every cell it requires.
    pub fn fits(&self, grid: usize) -> bool {
        self.cells
            .iter()
            .enumerate()
            .all(|(i, (_, role))| role.optional || self.cell_in(i, grid).is_some())
    }
}

fn required(accepts: &Ingredient) -> Role {
    Role {
        accepts: accepts.clone(),
        optional: false,
        patterned: None,
    }
}

/// A required cell whose stack must carry patterns, or must be blank.
fn patterned(accepts: &Ingredient, patterns: bool) -> Role {
    Role {
        patterned: Some(patterns),
        ..required(accepts)
    }
}

fn optional(accepts: &Ingredient) -> Role {
    Role {
        accepts: accepts.clone(),
        optional: true,
        patterned: None,
    }
}

/// Shapeless cells in order: their positions are only their order.
fn shapeless(roles: Vec<Role>) -> Vec<(usize, Role)> {
    roles.into_iter().enumerate().collect()
}

fn first_item(ingredient: &Ingredient) -> ItemStack {
    ItemStack::new(
        ingredient
            .order
            .first()
            .map_or("minecraft:barrier", String::as_str),
        1,
    )
}

/// The builders' templates, one per special crafting recipe the recipes hold, in their order.
pub fn templates(recipes: &Recipes) -> Vec<Template> {
    let mut out = Vec::new();
    for (name, recipe) in &recipes.crafting {
        let Recipe::Special(special) = recipe else {
            continue;
        };
        let result = |template: &crate::crafting::Template, or: &Ingredient| match &template.item {
            Some(_) => template.create(),
            None => first_item(or),
        };
        let (kind, icon, cells, shaped) = match special {
            Special::FireworkRocket {
                shell,
                fuel,
                star,
                result: made,
            } => (
                Kind::Fireworks,
                result(made, shell),
                // Paper and one gunpowder; more gunpowder flies higher, stars burst.
                shapeless(vec![
                    required(shell),
                    required(fuel),
                    optional(star),
                    optional(fuel),
                    optional(fuel),
                    optional(star),
                    optional(star),
                ]),
                false,
            ),
            Special::FireworkStar {
                dye,
                fuel,
                trail,
                twinkle,
                shapes,
                result: made,
            } => {
                let shape = Ingredient::of(
                    shapes
                        .iter()
                        .flat_map(|(_, i)| i.order.iter().cloned())
                        .collect(),
                );
                let mut roles = vec![
                    required(fuel),
                    required(dye),
                    optional(&shape),
                    optional(trail),
                    optional(twinkle),
                ];
                roles.extend((0..4).map(|_| optional(dye)));
                (Kind::Fireworks, result(made, fuel), shapeless(roles), false)
            }
            Special::FireworkStarFade {
                dye,
                target,
                result: made,
            } => {
                let mut roles = vec![required(target), required(dye)];
                roles.extend((0..7).map(|_| optional(dye)));
                (
                    Kind::Fireworks,
                    result(made, target),
                    shapeless(roles),
                    false,
                )
            }
            Special::Dye {
                target,
                dye,
                result: _,
            } => {
                let mut roles = vec![required(target), required(dye)];
                roles.extend((0..7).map(|_| optional(dye)));
                (Kind::Dyeing, first_item(target), shapeless(roles), false)
            }
            Special::Transmute {
                input,
                material,
                min,
                max,
                result: made,
                ..
            } => {
                let dyes = !material.order.is_empty()
                    && material.order.iter().all(|i| i.ends_with("_dye"));
                let mut roles = vec![required(input)];
                for n in 0..(*max).clamp(1, 8) {
                    roles.push(if n < (*min).max(1) {
                        required(material)
                    } else {
                        optional(material)
                    });
                }
                let kind = if dyes { Kind::Dyeing } else { Kind::Copying };
                (kind, result(made, input), shapeless(roles), false)
            }
            Special::BookCloning {
                source,
                material,
                min,
                max,
                result: made,
            } => {
                let mut roles = vec![required(source)];
                for n in 0..(*max).clamp(1, 8) {
                    roles.push(if n < (*min).max(1) {
                        required(material)
                    } else {
                        optional(material)
                    });
                }
                (Kind::Copying, result(made, source), shapeless(roles), false)
            }
            Special::MapExtending {
                map,
                material,
                result: made,
            } => {
                // The map in the middle, paper all round it.
                let cells = (0..9)
                    .map(|at| {
                        let role = if at == 4 { map } else { material };
                        (at, required(role))
                    })
                    .collect();
                (Kind::Copying, result(made, map), cells, true)
            }
            Special::BannerDuplicate {
                banner,
                result: made,
            } => (
                Kind::Banners,
                result(made, banner),
                // `BannerDuplicateRecipe`: one patterned banner to copy, one blank to take it.
                shapeless(vec![patterned(banner, true), patterned(banner, false)]),
                false,
            ),
            Special::ShieldDecoration {
                banner,
                target,
                result: made,
            } => (
                Kind::Banners,
                result(made, target),
                // `ShieldDecorationRecipe`: the shield must be blank.
                shapeless(vec![required(banner), patterned(target, false)]),
                false,
            ),
            Special::DecoratedPot {
                back,
                left,
                right,
                front,
                result: made,
            } => (
                Kind::Pottery,
                result(made, back),
                // The pattern " B ", "L R", " F ".
                vec![
                    (1, required(back)),
                    (3, required(left)),
                    (5, required(right)),
                    (7, required(front)),
                ],
                true,
            ),
            Special::Imbue {
                source,
                material,
                result: made,
            } => {
                // A lingering potion in the middle, arrows all round it.
                let cells = (0..9)
                    .map(|at| {
                        let role = if at == 4 { source } else { material };
                        (at, required(role))
                    })
                    .collect();
                (Kind::Arrows, result(made, material), cells, true)
            }
            Special::RepairItem => continue,
        };
        // Dyed armour comes a recipe per piece; the rest group by what their first cell takes.
        let family = match special {
            Special::Dye { .. } => "dye".to_owned(),
            // A recipe per banner colour: one entry.
            Special::BannerDuplicate { .. } => "banner_duplicate".to_owned(),
            _ => cells
                .first()
                .map(|(_, r)| r.accepts.order.join(","))
                .unwrap_or_default(),
        };
        out.push(Template {
            recipe: if name.contains(':') {
                name.to_string()
            } else {
                format!("minecraft:{name}")
            },
            kind,
            icon,
            cells,
            shaped,
            family,
        });
    }
    out
}

/// The templates as the row shows them: one entry per thing a builder works on (its first cell's
/// ingredient), its templates the entry's variants. Dyeing a shulker box takes a recipe per colour
/// in vanilla; here they're one entry with the colours in its variant column.
pub fn groups(templates: &[Template]) -> Vec<Vec<usize>> {
    let mut out: Vec<(Kind, &str, Vec<usize>)> = Vec::new();
    for (i, t) in templates.iter().enumerate() {
        match out
            .iter_mut()
            .find(|(k, family, _)| *k == t.kind && *family == t.family)
        {
            Some((_, _, members)) => members.push(i),
            None => out.push((t.kind, &t.family, vec![i])),
        }
    }
    out.into_iter().map(|(_, _, members)| members).collect()
}

/// The stacks of `inventory` that pass `test`, one of each kind (item and components) and one
/// item each, in the inventory's order: the hotbar first. What a builder offers to stage.
pub fn held_kinds(
    inventory: &[Option<ItemStack>],
    test: impl Fn(&ItemStack) -> bool,
) -> Vec<ItemStack> {
    let mut out: Vec<ItemStack> = Vec::new();
    for stack in inventory.iter().flatten() {
        if test(stack) && !out.iter().any(|s| s.same_item_same_components(stack)) {
            let mut one = stack.clone();
            one.count = 1;
            out.push(one);
        }
    }
    out
}

/// The stacks of `inventory` a cell can take (`held_kinds` of its ingredient).
pub fn candidates(
    context: &LoopContext,
    role: &Role,
    inventory: &[Option<ItemStack>],
) -> Vec<ItemStack> {
    held_kinds(inventory, |s| {
        role.accepts.test_in(context, s)
            && role.patterned.is_none_or(|patterned| {
                let layers = crate::banner::layers(s).len();
                if patterned {
                    // An ominous banner's eight layers are more than a copy takes.
                    (1..=crate::banner::MAX_LAYERS).contains(&layers)
                } else {
                    layers == 0
                }
            })
    })
}

/// The choice after (or before) `now` among `choices`, round the ends; the first when `now` isn't
/// one of them. None when there's no choice at all.
pub fn cycle(
    choices: &[Option<ItemStack>],
    now: Option<&ItemStack>,
    next: bool,
) -> Option<Option<ItemStack>> {
    let n = choices.len();
    if n == 0 {
        return None;
    }
    let at = choices.iter().position(|c| match (c, now) {
        (Some(a), Some(b)) => a.same_item_same_components(b),
        (None, None) => true,
        _ => false,
    });
    let to = match at {
        Some(at) => wrap(at, n, next),
        None => 0,
    };
    Some(choices[to].clone())
}

/// One step from `at` among `n`, round the ends.
pub fn wrap(at: usize, n: usize, forward: bool) -> usize {
    if n == 0 {
        return 0;
    }
    let at = at.min(n - 1);
    if forward {
        (at + 1) % n
    } else {
        (at + n - 1) % n
    }
}

/// How many of `stack` the inventory holds.
pub fn held(stack: &ItemStack, inventory: &[Option<ItemStack>]) -> u32 {
    inventory
        .iter()
        .flatten()
        .filter(|s| s.same_item_same_components(stack))
        .map(|s| s.count)
        .sum()
}

/// A first staging of a template's cells: each required cell takes the first candidate the
/// inventory still has one of; optional cells stay empty.
pub fn stage(
    context: &LoopContext,
    template: &Template,
    inventory: &[Option<ItemStack>],
) -> Vec<Option<ItemStack>> {
    let mut staged: Vec<Option<ItemStack>> = vec![None; template.cells.len()];
    for (i, (_, role)) in template.cells.iter().enumerate() {
        if role.optional {
            continue;
        }
        staged[i] = candidates(context, role, inventory).into_iter().find(|c| {
            let used = staged
                .iter()
                .flatten()
                .filter(|s| s.same_item_same_components(c))
                .count();
            held(c, inventory) as usize > used
        });
    }
    staged
}

/// A kept staging brought up to date with the inventory: each choice still held stays (the
/// player's picks, optional ones too), a choice no longer held goes, and a required cell left
/// empty takes the first candidate held, as `stage` would. A required ingredient that came back
/// (or was only in hand a moment) fills its cell again.
pub fn restage(
    context: &LoopContext,
    template: &Template,
    kept: &[Option<ItemStack>],
    inventory: &[Option<ItemStack>],
) -> Vec<Option<ItemStack>> {
    let mut staged: Vec<Option<ItemStack>> = vec![None; template.cells.len()];
    let room = |staged: &[Option<ItemStack>], c: &ItemStack| {
        let used = staged
            .iter()
            .flatten()
            .filter(|s| s.same_item_same_components(c))
            .count();
        held(c, inventory) as usize > used
    };
    for (i, choice) in kept.iter().enumerate().take(staged.len()) {
        if let Some(choice) = choice
            && room(&staged, choice)
        {
            staged[i] = Some(choice.clone());
        }
    }
    for (i, (_, role)) in template.cells.iter().enumerate() {
        if role.optional || staged[i].is_some() {
            continue;
        }
        staged[i] = candidates(context, role, inventory)
            .into_iter()
            .find(|c| room(&staged, c));
    }
    staged
}

/// Whether the inventory holds every staged item, as many times as it's staged.
pub fn available(staged: &[Option<ItemStack>], inventory: &[Option<ItemStack>]) -> bool {
    staged.iter().flatten().all(|stack| {
        let used = staged
            .iter()
            .flatten()
            .filter(|s| s.same_item_same_components(stack))
            .count();
        held(stack, inventory) as usize >= used
    })
}

/// The grid the staging lays out, `grid` wide: none when a staged cell has no room in it.
pub fn layout(
    template: &Template,
    staged: &[Option<ItemStack>],
    grid: usize,
) -> Option<Vec<Option<ItemStack>>> {
    let mut cells = vec![None; grid * grid];
    for (i, stack) in staged.iter().enumerate() {
        let Some(stack) = stack else {
            continue;
        };
        *cells.get_mut(template.cell_in(i, grid)?)? = Some(stack.clone());
    }
    Some(cells)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::Items;
    use std::path::{Path, PathBuf};

    fn ctx() -> &'static LoopContext {
        crate::test_resources::server_loop_context()
    }

    /// The real recipes. Asked for, the prepared resources must be there: a daily test fails
    /// without them rather than passing untried.
    fn recipes() -> Recipes {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION);
        let root: PathBuf = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(PathBuf::from)
            .unwrap_or(root);
        assert!(
            root.join("registries.json").exists(),
            "no prepared resources at {} (HORIZON_TEST_RESOURCES): run horizon-prepare",
            root.display()
        );
        let none = &horizon_world::loot::PackOverlay::NONE;
        let items = Items::catalog(
            &root,
            none,
            &horizon_world::item_defaults::read(&root).expect("item defaults"),
        )
        .expect("items");
        Recipes::load(&root, &items, none, &crate::test_resources::server_load()).expect("recipes")
    }

    fn stacks(items: &[(&str, u32)]) -> Vec<Option<ItemStack>> {
        items
            .iter()
            .map(|(i, n)| Some(ItemStack::new(i, *n)))
            .collect()
    }

    #[test]
    fn held_kinds_are_one_of_each_kind_in_inventory_order() {
        let inventory = vec![
            Some(ItemStack::new("minecraft:dirt", 5)),
            None,
            Some(ItemStack::new("minecraft:stone", 2)),
            Some(ItemStack::new("minecraft:dirt", 7)),
        ];
        let kinds = held_kinds(&inventory, |_| true);
        let items: Vec<&str> = kinds.iter().map(|s| &*s.item).collect();
        assert_eq!(items, ["minecraft:dirt", "minecraft:stone"]);
        assert!(kinds.iter().all(|s| s.count == 1));
    }

    #[test]
    fn cycle_steps_round_the_ends_and_through_nothing() {
        let a = ItemStack::new("minecraft:red_dye", 1);
        let b = ItemStack::new("minecraft:blue_dye", 1);
        let choices = [None, Some(a.clone()), Some(b.clone())];
        assert_eq!(cycle(&choices, None, true), Some(Some(a.clone())));
        assert_eq!(cycle(&choices, Some(&b), true), Some(None));
        assert_eq!(cycle(&choices, None, false), Some(Some(b.clone())));
        // Not among them: the first.
        let other = ItemStack::new("minecraft:dirt", 1);
        assert_eq!(cycle(&choices, Some(&other), true), Some(None));
        assert_eq!(cycle(&[], None, true), None);
        assert_eq!(wrap(0, 3, false), 2);
        assert_eq!(wrap(7, 3, true), 0);
    }

    #[test]
    #[ignore = "daily"]
    fn every_builder_has_a_template_from_the_real_recipes() {
        let recipes = recipes();
        let all = templates(&recipes);
        for kind in Kind::ALL {
            assert!(all.iter().any(|t| t.kind == kind), "no {kind:?} template");
        }
        let pot = all.iter().find(|t| t.kind == Kind::Pottery).unwrap();
        assert!(pot.shaped && !pot.fits(2) && pot.fits(3));
    }

    #[test]
    #[ignore = "daily"]
    fn colour_recipes_of_one_thing_share_a_row_entry() {
        let recipes = recipes();
        let all = templates(&recipes);
        let groups = groups(&all);
        let dyeing: Vec<&Vec<usize>> = groups
            .iter()
            .filter(|g| all[g[0]].kind == Kind::Dyeing)
            .collect();
        // Armour, shulker boxes, bundles...: a handful of entries, not a recipe per colour.
        // Armour, shulker boxes and bundles: three entries, not a recipe per colour or piece.
        assert_eq!(dyeing.len(), 3, "dyeing entries");
        let shulker = dyeing
            .iter()
            .find(|g| {
                all[g[0]].cells[0]
                    .1
                    .accepts
                    .order
                    .iter()
                    .any(|i| i.ends_with("shulker_box"))
            })
            .expect("shulker box dyeing");
        assert_eq!(shulker.len(), 16);
        let banners = groups
            .iter()
            .filter(|g| all[g[0]].kind == Kind::Banners)
            .count();
        assert_eq!(banners, 2, "copying a banner and decorating a shield");
        // Every template is in exactly one entry.
        let mut seen: Vec<usize> = groups.iter().flatten().copied().collect();
        seen.sort();
        assert_eq!(seen, (0..all.len()).collect::<Vec<_>>());
    }

    #[test]
    #[ignore = "daily"]
    fn a_required_cell_emptied_while_its_stack_was_away_fills_again() {
        let recipes = recipes();
        let all = templates(&recipes);
        let rocket = all
            .iter()
            .find(|t| &*t.icon.item == "minecraft:firework_rocket")
            .unwrap();
        // The gunpowder was in hand (a craft moving it): its cell was staged empty.
        let without = stacks(&[("minecraft:paper", 3)]);
        let staged = stage(ctx(), rocket, &without);
        assert_eq!(staged.iter().flatten().count(), 1);
        // Back in the inventory: the cell fills again; the paper choice stays.
        let inventory = stacks(&[("minecraft:paper", 3), ("minecraft:gunpowder", 5)]);
        let again = restage(ctx(), rocket, &staged, &inventory);
        let items: Vec<&str> = again.iter().flatten().map(|s| &*s.item).collect();
        assert_eq!(items, ["minecraft:paper", "minecraft:gunpowder"]);
        let grid = layout(rocket, &again, 2).unwrap();
        assert!(recipes.craft(ctx(), &grid, 2).is_some());
        // A pick no longer held goes; an optional cell left empty stays empty.
        let gone = restage(
            ctx(),
            rocket,
            &again,
            &stacks(&[("minecraft:gunpowder", 5)]),
        );
        assert!(
            gone.iter()
                .flatten()
                .all(|s| &*s.item == "minecraft:gunpowder")
        );
        assert_eq!(gone.iter().flatten().count(), 1);
    }

    #[test]
    #[ignore = "daily"]
    fn a_staged_rocket_makes_what_the_game_would() {
        let recipes = recipes();
        let all = templates(&recipes);
        let rocket = all
            .iter()
            .find(|t| &*t.icon.item == "minecraft:firework_rocket")
            .unwrap();
        let inventory = stacks(&[("minecraft:paper", 3), ("minecraft:gunpowder", 5)]);
        let staged = stage(ctx(), rocket, &inventory);
        assert!(available(&staged, &inventory));
        // Paper and one gunpowder, nothing optional.
        assert_eq!(staged.iter().flatten().count(), 2);
        let grid = layout(rocket, &staged, 2).unwrap();
        let made = recipes.craft(ctx(), &grid, 2).expect("a rocket");
        assert_eq!(&*made.item, "minecraft:firework_rocket");
        assert_eq!(made.count, 3);
    }

    #[test]
    #[ignore = "daily"]
    fn staging_never_uses_more_than_the_inventory_holds() {
        let recipes = recipes();
        let all = templates(&recipes);
        let copy = all
            .iter()
            .find(|t| t.kind == Kind::Banners && t.cells.len() == 2)
            .unwrap();
        // One banner can't fill both cells of a banner copy.
        let inventory = stacks(&[("minecraft:white_banner", 1)]);
        let staged = stage(ctx(), copy, &inventory);
        assert!(available(&staged, &inventory));
        assert!(staged.iter().flatten().count() < 2);
        let twice = vec![Some(ItemStack::new("minecraft:white_banner", 1)); 2];
        assert!(!available(&twice, &inventory));
    }

    #[test]
    #[ignore = "daily"]
    fn a_banner_copy_stages_a_copyable_patterned_banner_and_a_blank_one() {
        let recipes = recipes();
        let all = templates(&recipes);
        let copy = all
            .iter()
            .find(|t| {
                t.kind == Kind::Banners
                    && t.cells.iter().all(|(_, r)| {
                        r.accepts
                            .order
                            .iter()
                            .any(|i| i == "minecraft:white_banner")
                    })
            })
            .expect("the white banner copy");
        // Two blank banners first in the inventory, the patterned one after them.
        let blank = ItemStack::new("minecraft:white_banner", 1);
        let striped = crate::banner::loom_result(&blank, "red", "minecraft:stripe_bottom");
        // More layers than a copy takes (an ominous banner has eight), first of the patterned.
        let crowded = (0..=crate::banner::MAX_LAYERS).fold(blank.clone(), |b, _| {
            crate::banner::loom_result(&b, "black", "minecraft:stripe_top")
        });
        let inventory = vec![
            Some(blank.clone()),
            Some(blank),
            Some(crowded),
            Some(striped.clone()),
        ];
        let staged = stage(ctx(), copy, &inventory);
        assert_eq!(staged.iter().flatten().count(), 2, "{staged:?}");
        assert!(
            staged.iter().flatten().any(|s| *s == striped),
            "the patterned banner is staged: {staged:?}"
        );
        let grid = layout(copy, &staged, 2).unwrap();
        let made = recipes.craft(ctx(), &grid, 2).expect("a copy");
        assert_eq!(&*made.item, "minecraft:white_banner");
        assert_eq!(
            crate::banner::layers(&made),
            crate::banner::layers(&striped)
        );
    }

    #[test]
    #[ignore = "daily"]
    fn a_shaped_template_has_no_place_in_a_small_grid() {
        let recipes = recipes();
        let all = templates(&recipes);
        let pot = all.iter().find(|t| t.kind == Kind::Pottery).unwrap();
        let staged = vec![Some(ItemStack::new("minecraft:brick", 1)); 4];
        assert!(layout(pot, &staged, 2).is_none());
        let grid = layout(pot, &staged, 3).unwrap();
        assert!(grid[1].is_some() && grid[4].is_none() && grid[7].is_some());
        let made = recipes.craft(ctx(), &grid, 3).expect("a pot");
        assert_eq!(&*made.item, "minecraft:decorated_pot");
    }
}
