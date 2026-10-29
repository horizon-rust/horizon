use horizon_client::{
    crafting::Recipes,
    items::{ItemStack, Items},
    recipe_book::RecipeIndex,
    stations::StationRecipes,
};
use serde_json::json;
use std::{fs, path::Path};

fn write(root: &Path, name: &str, value: serde_json::Value) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec(&value).unwrap()).unwrap();
}

#[test]
fn loaders_keep_nested_recipe_ids_and_identical_basenames() {
    let root = std::env::temp_dir().join(format!("horizon-nested-recipes-{}", std::process::id()));
    fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
    write(&root, "items.json", json!({}));
    write(
        &root,
        "horizon-mod-content.json",
        json!(["pixelmon", "tcg"]),
    );
    for (path, ingredient, result) in [
        ("minecraft/recipe/same", "stone", "stick"),
        ("minecraft/recipe/tools/same", "dirt", "torch"),
        ("pixelmon/recipe/balls/same", "cobblestone", "apple"),
        ("tcg/recipe/cards/same", "gravel", "paper"),
    ] {
        write(
            &root,
            &format!("data/{path}.json"),
            json!({"type":"minecraft:crafting_shapeless", "ingredients":[format!("minecraft:{ingredient}")],
                "result":{"id":format!("minecraft:{result}"), "count":1}}),
        );
    }
    write(
        &root,
        "data/pixelmon/recipe/cooking/same.json",
        json!({"type":"minecraft:smelting", "ingredient":"minecraft:sand",
            "result":{"id":"minecraft:glass"}, "experience":0.1, "cookingtime":100}),
    );
    write(
        &root,
        "data/tcg/recipe/cutting/same.json",
        json!({"type":"minecraft:stonecutting", "ingredient":"minecraft:stone",
            "result":{"id":"minecraft:stone_slab", "count":2}}),
    );
    let items = Items::catalog(&root).unwrap();
    let recipes = Recipes::load(&root, &items).unwrap();
    for (ingredient, expected_id, result) in [
        ("stone", "same", "stick"),
        ("dirt", "tools/same", "torch"),
        ("cobblestone", "pixelmon:balls/same", "apple"),
        ("gravel", "tcg:cards/same", "paper"),
    ] {
        let grid = [Some(ItemStack::new(&format!("minecraft:{ingredient}"), 1))];
        let (id, stack) = recipes.find(&grid, 1).unwrap();
        assert_eq!(id, expected_id);
        assert_eq!(&*stack.item, format!("minecraft:{result}"));
    }
    let cooked = recipes
        .cook("minecraft:smelting", &ItemStack::new("minecraft:sand", 1))
        .unwrap();
    assert_eq!(cooked.id, "pixelmon:cooking/same");
    let stations = StationRecipes::load(&root, &items);
    let cuts = stations.stonecutter(&ItemStack::new("minecraft:stone", 1));
    assert_eq!(cuts.len(), 1);
    assert_eq!(cuts[0].id, "tcg:cutting/same");
    let index = RecipeIndex::load(&root);
    assert!(index.contains("minecraft:same"));
    assert!(index.contains("minecraft:tools/same"));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn raw_symlink_roots_are_skipped_but_content_overlays_resolve_their_vanilla_root() {
    let root = std::env::temp_dir().join(format!("horizon-linked-recipes-{}", std::process::id()));
    fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
    write(&root, "items.json", json!({}));
    write(
        &root,
        "original/stone.json",
        json!({"type":"minecraft:stonecutting", "ingredient":"minecraft:stone",
            "result":{"id":"minecraft:stone_slab"}}),
    );
    std::os::unix::fs::symlink(root.join("original"), root.join("data/minecraft/recipe")).unwrap();
    let items = Items::catalog(&root).unwrap();
    assert!(Recipes::load(&root, &items).unwrap().meta.is_empty());
    assert!(StationRecipes::load(&root, &items).stonecutting.is_empty());
    assert!(!RecipeIndex::load(&root).contains("minecraft:stone"));
    write(&root, "horizon-mod-content.json", json!(["pixelmon"]));
    assert!(
        Recipes::load(&root, &items)
            .unwrap()
            .meta
            .contains_key("stone")
    );
    assert_eq!(StationRecipes::load(&root, &items).stonecutting.len(), 1);
    assert!(RecipeIndex::load(&root).contains("minecraft:stone"));
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn unreadable_descendants_preserve_readable_loader_entries() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("horizon-denied-recipes-{}", std::process::id()));
    fs::create_dir_all(root.join("data/minecraft/loot_table")).unwrap();
    write(&root, "items.json", json!({}));
    for name in ["stone", "nested/stone"] {
        write(
            &root,
            &format!("data/minecraft/recipe/{name}.json"),
            json!({"type":"minecraft:stonecutting", "ingredient":"minecraft:stone",
                "result":{"id":"minecraft:stone_slab"}}),
        );
    }
    let blocked = root.join("data/minecraft/recipe/unreadable");
    fs::create_dir(&blocked).unwrap();
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o000)).unwrap();
    let denied = fs::read_dir(&blocked).is_err();
    let items = Items::catalog(&root).unwrap();
    let crafting = Recipes::load(&root, &items);
    let stations = StationRecipes::load(&root, &items);
    let index = RecipeIndex::load(&root);
    fs::set_permissions(&blocked, fs::Permissions::from_mode(0o755)).unwrap();
    fs::remove_dir_all(root).unwrap();
    if !denied {
        eprintln!("directory permission control unavailable for this user");
        return;
    }
    let crafting = crafting.unwrap();
    assert!(crafting.meta.contains_key("stone"));
    assert!(crafting.meta.contains_key("nested/stone"));
    assert_eq!(stations.stonecutting.len(), 2);
    assert!(index.contains("minecraft:stone"));
    assert!(index.contains("minecraft:nested/stone"));
}
