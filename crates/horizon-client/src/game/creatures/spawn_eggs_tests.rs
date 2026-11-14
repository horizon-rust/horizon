//! A spawn egg's own effects: `EntityType.spawn` plays the new mob's ambient
//! sound, `ItemStack.useOn` keeps an adventure player from using the egg on a
//! block, and an egg used on a mob of its own kind gives the baby the
//! tameness `getBreedOffspring` gives it.
use super::super::super::*;
use horizon_client::{
    command::args::GameMode, items::ItemStack, mobs::Species, motion::PlayerMotion,
};
use horizon_core::geometry::Vec3;
use horizon_world::nbt::Tag;

/// A server game with a local player standing south of `pos` and aiming at its
/// top face, the click forced onto that block, which holds `block`.
fn block_game(pos: [i32; 3], block: &str) -> Game<'static> {
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.set_allow_commands(true);
    // A local player, not a dedicated server's silent world: the egg reads the
    // spawn position from its crosshair.
    game.options.dedicated = None;
    let state = game.pack.assets.registry.default_state(block).unwrap();
    game.world.set(pos[0], pos[1], pos[2], state).unwrap();
    game.player = Some(
        PlayerMotion::new(
            game.motion,
            Vec3::new(
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]),
                f64::from(pos[2]) + 2.5,
            ),
        )
        .unwrap(),
    );
    game.screen = Screen::Playing;
    game.creative = false;
    game.ui.difficulty = 2;
    game.forced_hit = Some(Hit {
        pos,
        face: [0, 1, 0],
        point: [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 1.0,
            f64::from(pos[2]) + 0.5,
        ],
    });
    game
}

fn target(id: u32) -> Option<InteractTarget> {
    Some(InteractTarget {
        id: id as i32,
        location: [0.0, 0.0, 0.0],
    })
}

/// `EntityType.spawn` calls `Mob.playAmbientSound` on the mob it placed, so a
/// sheep egg used on a block calls out at the sheep's feet, at the mob's own
/// volume and an adult's pitch. (A pig's ambient sound depends on the sound
/// variant `finalizeSpawn` rolled for it; a sheep has only one.)
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_egg_used_on_a_block_plays_the_new_mobs_ambient_sound() {
    const POS: [i32; 3] = [2, 64, 2];
    let mut game = block_game(POS, "minecraft:stone");
    *game.held_mut() = Some(ItemStack::new("minecraft:sheep_spawn_egg", 2));
    game.audio.capture = Some(Vec::new());
    let hit = game.forced_hit.unwrap();
    assert!(game.use_hand(Some(&hit)).unwrap(), "the egg is used");
    let sounds = game.audio.capture.take().unwrap();
    let sheep = game
        .mobs
        .iter()
        .find(|m| m.kind.id == "minecraft:sheep")
        .expect("a sheep");
    let heard: Vec<_> = sounds
        .iter()
        .filter(|s| s.event == "minecraft:entity.sheep.ambient")
        .collect();
    assert_eq!(heard.len(), 1, "the sheep's ambient sound: {sounds:?}");
    assert_eq!(sounds.len(), 1, "one sound in all: {sounds:?}");
    assert_eq!(
        heard[0].position,
        [sheep.position.x, sheep.position.y, sheep.position.z],
        "`Entity.playSound` plays at the mob's feet"
    );
    assert_eq!(heard[0].volume, 1.0, "`Mob.getSoundVolume`");
    // `AgeableMob.getVoicePitch`: 1.0 plus the entity's own random, which a
    // baby raises to 1.5.
    let pitch = heard[0].pitch;
    assert!((0.8..=1.2).contains(&pitch), "an adult's pitch: {pitch}");
}

/// `ItemStack.useOn` passes in adventure mode unless the stack's `can_place_on`
/// names the clicked block, so an adventure player's egg spawns nothing and
/// stays in hand; a survival player's egg spawns, and so does an adventure
/// player's `can_place_on` egg on the block it names. That guard is the
/// stack's, so it answers before `EntityType.canSpawn` does.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_adventure_player_cannot_use_an_egg_on_a_block() {
    const POS: [i32; 3] = [2, 64, 2];
    let mut game = block_game(POS, "minecraft:stone");
    let hit = game.forced_hit.unwrap();
    *game.held_mut() = Some(ItemStack::new("minecraft:sheep_spawn_egg", 2));
    game.set_mode(GameMode::Adventure);
    assert!(
        !game.use_hand(Some(&hit)).unwrap(),
        "the use passes in adventure mode"
    );
    assert!(game.mobs.is_empty(), "no sheep: {:?}", game.mobs.len());
    assert_eq!(game.held().map(|s| s.count), Some(2), "the egg stayed");

    // `canPlaceOnBlockInAdventureMode`: the stack names the clicked block.
    let mut named = ItemStack::new("minecraft:sheep_spawn_egg", 2);
    named.data_mut().other.push((
        "minecraft:can_place_on".into(),
        Tag::compound([(
            "blocks",
            Tag::List(vec![Tag::String("minecraft:stone".into())]),
        )]),
    ));
    *game.held_mut() = Some(named);
    assert!(
        game.use_hand(Some(&hit)).unwrap(),
        "the named block allows it"
    );
    assert_eq!(game.mobs.len(), 1, "one sheep");
    assert_eq!(game.held().map(|s| s.count), Some(1), "one egg is used");

    // Survival builds: the egg is used as usual.
    game.set_mode(GameMode::Survival);
    *game.held_mut() = Some(ItemStack::new("minecraft:sheep_spawn_egg", 2));
    assert!(game.use_hand(Some(&hit)).unwrap(), "survival may build");
    assert_eq!(game.mobs.len(), 2, "a second sheep");
    assert_eq!(game.held().map(|s| s.count), Some(1), "one egg is used");

    // The guard is `ItemStack.useOn`'s, so it comes before `EntityType.canSpawn`:
    // in adventure mode on peaceful a zombie egg passes (PASS), where
    // `canSpawn` alone would fail it.
    game.set_mode(GameMode::Adventure);
    game.ui.difficulty = 0;
    *game.held_mut() = Some(ItemStack::new("minecraft:zombie_spawn_egg", 2));
    assert!(
        !game.use_hand(Some(&hit)).unwrap(),
        "peaceful does not make the use fail"
    );
    assert!(!game.use_failed, "a pass, not a fail");
    assert_eq!(game.mobs.len(), 2, "no zombie: {}", game.mobs.len());
    assert_eq!(game.held().map(|s| s.count), Some(2), "the egg stayed");
}

/// `SpawnEggItem.spawnOffspringFromSpawnEgg` asks the parent for
/// `getBreedOffspring(level, this)`: a tame wolf's or cat's baby is tame, its
/// owner's, and wears the mix of its collar colour with itself. The wolf's pup
/// also takes `Wolf.getBreedOffspring`'s draws from the parent's own random
/// (the coat coin, then its voice), and the collar mix takes the level's.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_egg_on_your_own_wolf_gives_a_tame_pup_of_yours() {
    use horizon_core::random::{EntityRandom, LegacyRandom, RandomSource};
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.options.dedicated = None;
    game.creative = false;
    let owner = game.acting_player_uuid();
    let seed = 4_242i64;
    let voices = horizon_client::mobs::finalize::sound_variants(Species::Wolf).len() as i32;
    assert!(voices > 1, "a wolf has several voices");
    for (kind, id) in [("minecraft:wolf", 7u32), ("minecraft:cat", 8)] {
        let t = game.res.mob_types().get(kind).unwrap().clone();
        let mut parent = Mob::new(id, t, Vec3::new(2.5, 64.0, 2.5), 0.0);
        parent.random = EntityRandom::own(LegacyRandom::new(seed));
        parent.extra.tame = true;
        parent.extra.owner = Some(owner);
        parent.extra.color = 2;
        game.mobs.clear();
        game.mobs.push(parent);
        // Ids the spawner hands out stay clear of the parent's.
        game.next_entity = 1_000 + id;
        let egg = format!("{kind}_spawn_egg");
        *game.held_mut() = Some(ItemStack::new(&egg, 2));
        let mut level = game.world.random.clone();
        assert!(
            game.interact_mob(target(id)).unwrap(),
            "{kind}: the egg is used"
        );
        let child = game
            .mobs
            .iter()
            .find(|m| m.id != id)
            .unwrap_or_else(|| panic!("{kind}: a baby"));
        assert!(child.is_baby(), "{kind}: a baby");
        assert!(child.extra.tame, "{kind}: the baby is tame");
        assert_eq!(child.extra.owner, Some(owner), "{kind}: and its owner's");
        assert_eq!(
            child.extra.color, 2,
            "{kind}: the collar colour mixed with itself"
        );
        assert_eq!(game.held().map(|s| s.count), Some(1), "{kind}: one egg");
        // The parent's own stream: the coat coin, then a wolf's voice.
        let mut expected = LegacyRandom::new(seed);
        let _ = expected.next_boolean();
        let voice =
            (kind == "minecraft:wolf").then(|| expected.next_int_bound(voices).unwrap() as u8);
        assert_eq!(
            child.extra.sound_variant, voice,
            "{kind}: the parent's own random picks the voice"
        );
        // The collar mix is `DyeColor.getMixedColor(level, c, c)`: no recipe
        // mixes a colour with itself, so the level's random answers it.
        let _ = level.next_boolean();
        assert_eq!(
            game.world.random.state(),
            level.state(),
            "{kind}: one level draw, the colour mix"
        );
    }
}
