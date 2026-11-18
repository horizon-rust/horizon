//! The controller crafting screen's model (Horizon's own, off by default; docs/controllers.md):
//! the crafting recipes the player knows, sorted into vanilla's creative tabs, in creative order.
//!
//! Each tab lists the book's recipe collections (one per recipe group, or per ungrouped recipe,
//! as vanilla's recipe book groups them) whose result the tab shows. A result in several tabs goes
//! in the first. A result in none goes to the tab that stands for its recipe book category
//! ([`fallback_tab`]), so every recipe the book shows is in exactly one tab. Mods' creative tabs
//! come after vanilla's, as they do in the creative screen, so a mod's items get a tab of their own
//! with nothing else to declare.
//!
//! Nothing here is drawn or sent: `game/legacy_craft.rs` reads it to draw and to craft.
use crate::{
    creative,
    items::ItemStack,
    recipe_book::{Book, BookType, Category, ClientBook, Collection, Tab},
};
use std::collections::HashMap;

/// One tab of the screen.
#[derive(Clone, Debug)]
pub struct CraftTab {
    /// The creative tab's id (`minecraft:building_blocks`), or [`SEARCH`].
    pub id: String,
    /// The creative tab's title as the creative screen shows it.
    pub title: String,
    /// The item drawn on the tab.
    pub icon: String,
    /// The recipe collections, in the order the row shows them.
    pub groups: Vec<Collection>,
}

/// The search tab's id: its groups are the matches of what the player typed.
pub const SEARCH: &str = "horizon:search";

/// The creative tab a recipe goes to when no creative tab shows its result: the one closest to
/// its recipe book category.
pub fn fallback_tab(category: Category) -> &'static str {
    match category {
        Category::CraftingBuilding => "minecraft:building_blocks",
        Category::CraftingRedstone => "minecraft:redstone_blocks",
        Category::CraftingEquipment => "minecraft:tools_and_utilities",
        _ => "minecraft:ingredients",
    }
}

/// The item tags whose members are tiers of one thing (wooden to netherite): with tiers grouped,
/// their recipes share one collection, so the variant column steps through the tiers. Tags, so a
/// mod's tools join their kind.
pub const TIER_TAGS: [&str; 10] = [
    "minecraft:swords",
    "minecraft:spears",
    "minecraft:pickaxes",
    "minecraft:axes",
    "minecraft:shovels",
    "minecraft:hoes",
    "minecraft:head_armor",
    "minecraft:chest_armor",
    "minecraft:leg_armor",
    "minecraft:foot_armor",
];

/// Item to its tier tag, from the item tags (`Items::item_tag_order`): none when tiers aren't grouped.
pub fn tiers(tags: &HashMap<String, Vec<String>>) -> HashMap<String, &'static str> {
    let mut out = HashMap::new();
    for tag in TIER_TAGS {
        let name = tag.trim_start_matches("minecraft:");
        let Some(items) = tags.get(tag).or_else(|| tags.get(name)) else {
            continue;
        };
        for item in items {
            out.entry(item.clone()).or_insert(tag);
        }
    }
    out
}

/// Creative tabs that never list recipes: the search, saved toolbars, the survival inventory.
fn lists_recipes(tab: &creative::Tab) -> bool {
    !(tab.search || tab.hotbar || tab.inventory)
}

/// Where each item first shows in the creative tabs: its tab's index and its place in that tab.
fn creative_places(tabs: &[creative::Tab]) -> HashMap<&str, (usize, usize)> {
    let mut places = HashMap::new();
    for (t, tab) in tabs
        .iter()
        .enumerate()
        .filter(|(_, tab)| lists_recipes(tab))
    {
        for (i, stack) in tab.items.iter().enumerate() {
            places.entry(&*stack.item).or_insert((t, i));
        }
    }
    places
}

/// The tabs for a crafting grid `grid` wide (2 in the inventory, 3 at a crafting table): the
/// collections of the recipes the client knows that fit it, each in its result's creative tab,
/// and the tabs with none left out. The search tab comes first, with every collection.
pub fn tabs(
    book: &Book,
    client: &ClientBook,
    creative_tabs: &[creative::Tab],
    grid: usize,
    tiers: &HashMap<String, &'static str>,
) -> Vec<CraftTab> {
    let places = creative_places(creative_tabs);
    let tab_index: HashMap<&str, usize> = creative_tabs
        .iter()
        .enumerate()
        .map(|(i, t)| (&*t.id, i))
        .collect();
    // (creative tab, place in it, the book's order) for each collection.
    let mut placed: Vec<(usize, usize, usize, Collection)> = Vec::new();
    let mut lost = Vec::new();
    // A tier tag's collection in `placed`, so the next tier joins it.
    let mut tier_at: HashMap<&str, usize> = HashMap::new();
    let all = client.collections(book, BookType::Crafting, Tab::Search);
    for (order, collection) in all.into_iter().enumerate() {
        let entries: Vec<usize> = collection
            .entries
            .into_iter()
            .filter(|&e| book.entries[e].fits(grid, grid))
            .collect();
        let Some(&first) = entries.first() else {
            continue;
        };
        let entry = &book.entries[first];
        let place = entries
            .iter()
            .filter_map(|&e| places.get(&*book.entries[e].result.item).copied())
            .min()
            .or_else(|| {
                tab_index
                    .get(fallback_tab(entry.category))
                    .map(|&t| (t, usize::MAX))
            });
        let tier = tiers.get(&*entry.result.item).copied();
        if let (Some(tag), Some(place)) = (tier, place)
            && let Some(&i) = tier_at.get(tag)
        {
            let joined = &mut placed[i];
            (joined.0, joined.1) = (joined.0, joined.1).min(place);
            joined.3.entries.extend(entries);
            continue;
        }
        let collection = Collection { entries };
        match place {
            Some((tab, at)) => {
                if let Some(tag) = tier {
                    tier_at.insert(tag, placed.len());
                }
                placed.push((tab, at, order, collection))
            }
            None => lost.push(collection),
        }
    }
    // A tier column goes in creative order: wooden, stone, copper, iron, golden, diamond, netherite.
    let place_of = |e: usize| places.get(&*book.entries[e].result.item).copied();
    for &i in tier_at.values() {
        placed[i].3.entries.sort_by_key(|&e| place_of(e));
    }
    placed.sort_by_key(|&(tab, at, order, _)| (tab, at, order));
    let mut out: Vec<CraftTab> = Vec::new();
    let search = CraftTab {
        id: SEARCH.to_owned(),
        title: String::new(),
        icon: "minecraft:compass".to_owned(),
        groups: placed
            .iter()
            .map(|(_, _, _, c)| c.clone())
            .chain(lost.iter().cloned())
            .collect(),
    };
    out.push(search);
    for (tab, _, _, collection) in placed {
        let creative = &creative_tabs[tab];
        match out.last_mut() {
            Some(last) if last.id == creative.id => last.groups.push(collection),
            _ => out.push(CraftTab {
                id: creative.id.clone(),
                title: creative.title.clone(),
                icon: creative.icon.clone(),
                groups: vec![collection],
            }),
        }
    }
    // Only a creative export without the four fallback tabs leaves any: they stay searchable.
    if !lost.is_empty() {
        log::debug!("{} recipe collections have no creative tab", lost.len());
    }
    out
}

/// The collections of `groups` whose results' names hold every word of `query`, case folded,
/// as the recipe book's search matches (`name` gives an item's shown name).
pub fn search(
    book: &Book,
    groups: &[Collection],
    query: &str,
    name: &dyn Fn(&ItemStack) -> String,
) -> Vec<Collection> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return groups.to_vec();
    }
    groups
        .iter()
        .filter(|c| {
            c.entries.iter().any(|&e| {
                book.entries[e].results().iter().any(|r| {
                    let shown = name(r).to_lowercase();
                    words.iter().all(|w| shown.contains(w.as_str()))
                })
            })
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{crafting::Recipes, items::Items, recipe_book::RecipeIndex};
    use std::path::{Path, PathBuf};

    fn root() -> Option<PathBuf> {
        let root = std::env::var_os("HORIZON_TEST_RESOURCES")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../.local/horizon-client")
                    .join(horizon_core::VANILLA_VERSION)
            });
        root.join("registries.json").exists().then_some(root)
    }

    /// The real book with every recipe known, and the real creative tabs.
    fn vanilla() -> Option<(Book, ClientBook, creative::Creative)> {
        let root = root()?;
        let none = &horizon_world::loot::PackOverlay::NONE;
        // With the data here, failing to read it is a failure, not a skip.
        let items = Items::catalog(
            &root,
            none,
            &horizon_world::item_defaults::read(&root).expect("item defaults"),
        )
        .expect("items");
        let recipes = Recipes::load(&root, &items, none, &crate::test_resources::server_load())
            .expect("recipes");
        let mut book = Book::build(&recipes, &items);
        book.set_display_ids(&RecipeIndex::load(&root, none));
        let mut client = ClientBook::default();
        for e in &book.entries {
            if e.display_id >= 0 && e.category.name().starts_with("crafting") {
                client.add(e.display_id);
            }
        }
        let creative = creative::Creative::load(&root).expect("creative tabs");
        Some((book, client, creative))
    }

    fn entries_of(tab: &CraftTab) -> Vec<usize> {
        tab.groups.iter().flat_map(|c| c.entries.clone()).collect()
    }

    #[test]
    fn every_known_recipe_that_fits_is_in_exactly_one_tab() {
        let Some((book, client, creative)) = vanilla() else {
            return;
        };
        for grid in [2, 3] {
            let tabs = tabs(&book, &client, &creative.tabs, grid, &HashMap::new());
            let mut seen: Vec<usize> = tabs[1..].iter().flat_map(entries_of).collect();
            let count = seen.len();
            seen.sort();
            seen.dedup();
            assert_eq!(seen.len(), count, "a recipe in two tabs at grid {grid}");
            let expected: Vec<usize> = {
                let mut all: Vec<usize> = client
                    .collections(&book, BookType::Crafting, Tab::Search)
                    .into_iter()
                    .flat_map(|c| c.entries)
                    .filter(|&e| book.entries[e].fits(grid, grid))
                    .collect();
                all.sort();
                all
            };
            assert_eq!(
                seen, expected,
                "recipes missing from the tabs at grid {grid}"
            );
            assert!(
                expected.len() > 500,
                "the real book has hundreds of crafting recipes"
            );
            let mut search = entries_of(&tabs[0]);
            search.sort();
            assert_eq!(search, expected, "the search tab lists every recipe");
        }
    }

    #[test]
    fn results_land_in_their_creative_tab_in_creative_order() {
        let Some((book, client, creative)) = vanilla() else {
            return;
        };
        let tabs = tabs(&book, &client, &creative.tabs, 3, &HashMap::new());
        let tab_of = |item: &str| {
            tabs[1..]
                .iter()
                .find(|t| {
                    t.groups.iter().any(|c| {
                        c.entries
                            .iter()
                            .any(|&e| &*book.entries[e].result.item == item)
                    })
                })
                .map(|t| t.id.as_str())
        };
        assert_eq!(
            tab_of("minecraft:crafting_table"),
            Some("minecraft:functional_blocks")
        );
        assert_eq!(tab_of("minecraft:diamond_sword"), Some("minecraft:combat"));
        assert_eq!(tab_of("minecraft:bread"), Some("minecraft:food_and_drinks"));
        assert_eq!(
            tab_of("minecraft:piston"),
            Some("minecraft:redstone_blocks")
        );
        assert_eq!(tab_of("minecraft:stick"), Some("minecraft:ingredients"));
        // Stone bricks come before bricks, as in the Building Blocks tab.
        let building = tabs
            .iter()
            .find(|t| t.id == "minecraft:building_blocks")
            .unwrap();
        let position = |item: &str| {
            building.groups.iter().position(|c| {
                c.entries
                    .iter()
                    .any(|&e| &*book.entries[e].result.item == item)
            })
        };
        assert!(position("minecraft:stone_bricks") < position("minecraft:bricks"));
        // A group is one collection: every wooden stairs recipe in one place.
        assert_eq!(
            position("minecraft:oak_stairs"),
            position("minecraft:spruce_stairs")
        );
    }

    #[test]
    fn the_inventory_grid_leaves_out_recipes_bigger_than_two_by_two() {
        let Some((book, client, creative)) = vanilla() else {
            return;
        };
        let small = tabs(&book, &client, &creative.tabs, 2, &HashMap::new());
        let has = |item: &str| {
            entries_of(&small[0])
                .iter()
                .any(|&e| &*book.entries[e].result.item == item)
        };
        assert!(has("minecraft:crafting_table"));
        assert!(!has("minecraft:chest"));
    }

    #[test]
    fn every_banner_colour_is_on_a_crafting_table_tab_and_not_in_the_inventory() {
        let Some((book, client, creative)) = vanilla() else {
            return;
        };
        let on_tabs = |grid: usize, item: &str| {
            tabs(&book, &client, &creative.tabs, grid, &HashMap::new())[1..]
                .iter()
                .flat_map(entries_of)
                .any(|e| &*book.entries[e].result.item == item)
        };
        for colour in [
            "white",
            "orange",
            "magenta",
            "light_blue",
            "yellow",
            "lime",
            "pink",
            "gray",
            "light_gray",
            "cyan",
            "purple",
            "blue",
            "brown",
            "green",
            "red",
            "black",
        ] {
            let banner = format!("minecraft:{colour}_banner");
            assert!(on_tabs(3, &banner), "{banner} on no crafting table tab");
            // Six wool and a stick fill a 3x3 grid.
            assert!(!on_tabs(2, &banner), "{banner} in the inventory's 2x2");
        }
    }

    #[test]
    fn grouped_tiers_put_every_pickaxe_in_one_column_in_creative_order() {
        let Some((book, client, creative)) = vanilla() else {
            return;
        };
        let root = root().unwrap();
        let none = &horizon_world::loot::PackOverlay::NONE;
        let items = Items::catalog(
            &root,
            none,
            &horizon_world::item_defaults::read(&root).expect("item defaults"),
        )
        .expect("items");
        let tiers = tiers(&items.item_tag_order);
        assert_eq!(
            tiers.get("minecraft:iron_pickaxe"),
            Some(&"minecraft:pickaxes")
        );
        let tabs = tabs(&book, &client, &creative.tabs, 3, &tiers);
        let columns: Vec<Vec<String>> = tabs[1..]
            .iter()
            .flat_map(|t| &t.groups)
            .map(|c| {
                c.entries
                    .iter()
                    .map(|&e| book.entries[e].result.item.to_string())
                    .collect()
            })
            .filter(|c: &Vec<String>| c.iter().any(|i| i.ends_with("_pickaxe")))
            .collect();
        assert_eq!(columns.len(), 1, "pickaxes split: {columns:?}");
        let column = &columns[0];
        let at = |item: &str| column.iter().position(|i| i == item);
        assert!(at("minecraft:wooden_pickaxe") < at("minecraft:stone_pickaxe"));
        assert!(at("minecraft:stone_pickaxe") < at("minecraft:iron_pickaxe"));
        assert!(at("minecraft:iron_pickaxe") < at("minecraft:diamond_pickaxe"));
        // Still every recipe in exactly one tab.
        let mut seen: Vec<usize> = tabs[1..].iter().flat_map(entries_of).collect();
        let count = seen.len();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), count);
    }

    #[test]
    fn search_matches_every_word_of_the_shown_name() {
        let Some((book, client, creative)) = vanilla() else {
            return;
        };
        let tabs = tabs(&book, &client, &creative.tabs, 3, &HashMap::new());
        let name = |s: &ItemStack| s.item.trim_start_matches("minecraft:").replace('_', " ");
        let found = search(&book, &tabs[0].groups, "OAK  stairs", &name);
        assert!(!found.is_empty());
        for c in &found {
            assert!(c.entries.iter().any(|&e| {
                let n = name(&book.entries[e].result);
                n.contains("oak") && n.contains("stairs")
            }));
        }
        assert_eq!(
            search(&book, &tabs[0].groups, "  ", &name).len(),
            tabs[0].groups.len()
        );
    }
}
