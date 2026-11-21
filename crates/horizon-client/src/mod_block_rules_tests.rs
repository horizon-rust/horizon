//! A content block's vanilla rules come from its template: which block entity a
//! container's title, menu and slots are asked about, and which classes a
//! falling anvil, a shulker box's slots and a wind-charged door are asked about
//! (#18502). A block whose id happens to end in a vanilla one's name is not that
//! block, and a block of a vanilla template with an id of its own still is.
use crate::mod_content::ContentMod;
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

/// The vanilla data plus a content mod whose blocks copy templates only: a
/// chest, a barrel, a hopper, a dispenser, a shulker box, an anvil, doors and a
/// trapdoor, and stone blocks whose ids end in vanilla containers' and a
/// door's names. `None` without the prepared vanilla data (CI).
fn resources() -> Option<&'static Path> {
    static ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
    ROOT.get_or_init(|| {
        let base =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3");
        if !base.join("block-behaviours.json").exists() {
            return None;
        }
        let scratch = std::env::temp_dir().join(format!("block-rules-{}", std::process::id()));
        let dir = scratch.join("mod");
        let mut files = vec![(
            "horizon-mod.toml".to_owned(),
            "id = \"fieldtest\"\nname = \"Field test\"\nversion = \"1.0\"\napi = \"0.2\"\ncapabilities = [\"registries\"]\n".to_owned(),
        )];
        for (id, template) in [
            ("crate_of_stuff", "chest"),
            ("crate", "barrel"),
            ("chute", "hopper"),
            ("trap", "dispenser"),
            ("lockbox", "shulker_box"),
            ("hammer", "anvil"),
            ("fake_copper_chest", "stone"),
            ("iron_gate", "iron_door"),
            ("oak_gate", "oak_door"),
            ("copper_hatch", "copper_trapdoor"),
            ("fake_door", "stone"),
        ] {
            files.push((
                format!("data/fieldtest/horizon/block/{id}.json"),
                format!(r#"{{"template":"minecraft:{template}"}}"#),
            ));
        }
        for (name, text) in files {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let package = horizon_mod_package::Package::read(&dir).unwrap();
        let content = horizon_mod_package::content::Content::read(&dir, &package.manifest).unwrap();
        let out = scratch.join("resources");
        let mods = [ContentMod {
            id: "fieldtest".into(),
            content,
            jars: Vec::new(),
        }];
        let inventory = horizon_mod_package::enabled::EnabledMods::new(
            mods.iter()
                .map(|package| (package.id.clone(), mod_api::Side::Both)),
        );
        crate::mod_content::build(&base, Path::new(""), &mods, &out, &inventory).unwrap();
        Some(out)
    })
    .as_deref()
}

#[test]
fn a_content_blocks_rules_come_from_its_template_and_never_from_its_id() {
    let Some(root) = resources() else {
        return;
    };
    let assets = crate::assets::Assets::load(root).unwrap();
    let registry = assets.registry;
    let behaviours = horizon_world::level::Behaviours::load(registry, root).unwrap();
    let state = |name: &str| {
        registry
            .default_state(name)
            .unwrap_or_else(|| panic!("{name}"))
    };
    // What breaking the block does with its entity, which is what the rules
    // above decide: a chest-template block drops its 27 slots, a shulker box
    // keeps its contents for the dropped item, and the stone block named
    // `fake_copper_chest` has nothing to drop.
    use horizon_world::level::removal::{PreRemove, pre_remove};
    let contents = |slots| PreRemove::Contents {
        slots,
        furnace: false,
    };
    assert_eq!(
        pre_remove(&behaviours, state("fieldtest:crate_of_stuff")),
        contents(27)
    );
    assert_eq!(
        pre_remove(&behaviours, state("minecraft:oxidized_copper_chest")),
        contents(27)
    );
    assert_eq!(
        pre_remove(&behaviours, state("fieldtest:lockbox")),
        PreRemove::ShulkerBox
    );
    assert_eq!(
        pre_remove(&behaviours, state("fieldtest:fake_copper_chest")),
        PreRemove::Nothing
    );
    // A barrel's, a hopper's and a dispenser's slots drop by the block entity's
    // class, so a content block of each template keeps its contents.
    for (block, dropped) in [
        ("fieldtest:crate", contents(27)),
        ("fieldtest:chute", contents(5)),
        ("fieldtest:trap", contents(9)),
        ("minecraft:barrel", contents(27)),
        ("minecraft:hopper", contents(5)),
        ("minecraft:dispenser", contents(9)),
        ("minecraft:dropper", contents(9)),
        ("minecraft:crafter", contents(9)),
        ("minecraft:brewing_stand", contents(5)),
        ("minecraft:chiseled_bookshelf", contents(6)),
        ("minecraft:oak_shelf", contents(3)),
        ("minecraft:decorated_pot", contents(1)),
        ("minecraft:bookshelf", PreRemove::Nothing),
        (
            "minecraft:furnace",
            PreRemove::Contents {
                slots: 3,
                furnace: true,
            },
        ),
        ("minecraft:campfire", PreRemove::Campfire),
        ("minecraft:jukebox", PreRemove::Jukebox),
        ("minecraft:lectern", PreRemove::Lectern),
        ("minecraft:sculk_shrieker", PreRemove::SculkShrieker),
        ("minecraft:creaking_heart", PreRemove::CreakingHeart),
        ("minecraft:beacon", PreRemove::Beacon),
    ] {
        assert_eq!(pre_remove(&behaviours, state(block)), dropped, "{block}");
    }
    // A hand and a wind charge's burst ask the block's class and, for a door or
    // trapdoor, its `BlockSetType` flags (`BlockBehaviour.onExplosionHit`): a
    // content block of a door's template answers for it, and a stone block
    // whose id only ends in `_door` answers for nothing.
    for (block, by_wind, by_hand) in [
        ("minecraft:oak_door", true, true),
        ("minecraft:iron_door", false, false),
        ("minecraft:copper_trapdoor", true, true),
        ("minecraft:iron_trapdoor", false, false),
        ("minecraft:oak_fence_gate", true, true),
        ("minecraft:oak_button", true, true),
        ("minecraft:lever", true, true),
        ("minecraft:bell", true, false),
        ("minecraft:water_cauldron", false, true),
        ("fieldtest:iron_gate", false, false),
        ("fieldtest:oak_gate", true, true),
        ("fieldtest:copper_hatch", true, true),
        ("fieldtest:fake_door", false, false),
    ] {
        let data = behaviours.data(state(block));
        assert_eq!(
            horizon_world::level::flip::flips_by_wind_charge(data),
            by_wind,
            "{block} in a wind charge's burst"
        );
        assert_eq!(
            horizon_world::level::flip::flips_by_hand(data),
            by_hand,
            "{block} by hand"
        );
    }
    // A falling anvil's damage source asks the class, its saved `HurtEntities`
    // default and its wear ask the tag: vanilla's three anvils answer both, and
    // a content block of the anvil template is only the class until it adds
    // itself to `minecraft:anvil`.
    for anvil in [
        "minecraft:anvil",
        "minecraft:chipped_anvil",
        "minecraft:damaged_anvil",
    ] {
        assert!(behaviours.is_anvil(state(anvil)), "{anvil}");
        assert!(behaviours.is_anvil_tag(state(anvil)), "{anvil}");
    }
    assert!(behaviours.is_anvil(state("fieldtest:hammer")));
    assert!(!behaviours.is_anvil_tag(state("fieldtest:hammer")));
}
