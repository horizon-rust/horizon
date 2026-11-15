//! `LivingEntity.baseTick` forgets `lastHurtByPlayer` once its memory time has
//! run out, and `Raider.die` / `Villager.die` take the causing entity: a mob
//! killed long after the last hit, or by lava, is nobody's kill.
use super::super::*;
use horizon_client::chunk::Chunk;
use horizon_client::mobs::villager_brain;
use horizon_core::position::ChunkPos;
use horizon_core::random::{LegacyRandom, LevelRandom};
use std::sync::Arc;

/// The pigs a death is measured on, and the villagers of the murder.
const PIG_A: &str = "00006097-0000-0000-0000-000000000001";
const PIG_B: &str = "00006097-0000-0000-0000-000000000002";
const VICTIM: &str = "00006097-0000-0000-0000-000000000003";
const WITNESS: &str = "00006097-0000-0000-0000-000000000004";
const MURDERED: &str = "00006097-0000-0000-0000-000000000005";
const PLAYER: [i32; 4] = [0, 0, 0, 101];

fn game() -> Game<'static> {
    let root = std::path::PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES")
            .expect("set HORIZON_TEST_RESOURCES to prepared resources"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        std::time::Instant::now(),
        false,
        true,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let res: &'static _ = Box::leak(Box::new(loaded.resources));
    let mut game = Game::new(
        res,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    game.build_registries();
    game.set_allow_commands(true);
    // Entities tick once the 2 columns around their chunk are loaded
    // (`ChunkMap.prepareEntityTickingChunk`), so the chunk of the mobs has all of them.
    for x in -2..=2 {
        for z in -2..=2 {
            game.world
                .chunks
                .insert((x, z), Arc::new(Chunk::empty(x, z)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(x, z))
                .unwrap();
        }
    }
    for command in [
        "forceload add 0 0",
        "fill 0 99 0 15 99 15 stone",
        "tick freeze",
    ] {
        assert!(game.run_command(command), "{command}");
    }
    game.level.player_uuid = Some(PLAYER);
    // A player's counters are read from disk once, before anything is awarded
    // (`ServerStatsCounter.parseLocal`): a fresh test game has read them.
    game.stats.loaded = true;
    game
}

fn step(game: &mut Game<'_>, ticks: u32) {
    for _ in 0..ticks {
        assert!(game.run_command("tick step 1"));
        game.tick_server(false).unwrap();
    }
}

/// The mob summoned with `uuid`, by its place in `game.mobs`.
fn index(game: &mut Game<'_>, uuid: &str) -> usize {
    match game.target_with_uuid(horizon_client::save::uuid_from_text(uuid).unwrap()) {
        Some(super::commands::Target::Mob(id)) => {
            game.mobs.iter().position(|m| m.id == id).unwrap()
        }
        other => panic!("{other:?}"),
    }
}

/// A player's hit, as `/damage <mob> 1 minecraft:player_attack by @p` runs it:
/// `LivingEntity.hurtServer`'s `resolvePlayerResponsibleForDamage` sets
/// `lastHurtByPlayer(player, 100)` and the reference it keeps.
fn hit_by_player(game: &mut Game<'_>, uuid: &str, amount: f32) {
    let Some(super::commands::Target::Mob(id)) =
        game.target_with_uuid(horizon_client::save::uuid_from_text(uuid).unwrap())
    else {
        panic!("no mob {uuid}");
    };
    assert!(
        game.damage_target(
            super::commands::Target::Mob(id),
            amount,
            "minecraft:player_attack",
            Some(super::commands::Target::Player),
            None,
            Some([0.5, 100.0, 0.5]),
        ),
        "the player's hit lands"
    );
}

/// The victim's sensor sees the witness (`NEAREST_VISIBLE_LIVING_ENTITIES`),
/// as `Villager.die`'s `tellWitnessesThatIWasMurdered` reads it: the brain ticks
/// 20 times with the victim out of the level.
fn let_sees_witness(game: &mut Game<'_>, victim: usize, witness: u32) {
    let mut victim_mob = game.mobs.remove(victim);
    let snapshot: Vec<_> = game.mobs.iter().map(|m| m.seen(None)).collect();
    {
        let world = game.mob_world_base(&snapshot);
        let mut random = LevelRandom::for_level(LegacyRandom::new(0));
        let mut events = Vec::new();
        for _ in 0..20 {
            villager_brain::tick(&mut victim_mob, &world, &mut random, &mut events);
        }
        assert_eq!(
            villager_brain::murder_witnesses(&mut victim_mob, &world),
            vec![witness],
            "the other villager saw it"
        );
    }
    game.mobs.insert(victim, victim_mob);
}

/// The `killed` statistic of a pig: what the kill credit awards.
fn pigs_killed(game: &Game<'_>) -> i32 {
    game.stats.get("killed", "minecraft:pig")
}

fn summon(game: &mut Game<'_>, kind: &str, at: &str, n: i32, health: f32) {
    assert!(game.run_command(&format!(
        "summon {kind} {at} {{NoAI:1b,NoGravity:1b,PersistenceRequired:1b,Health:{health}f,UUID:[I;24727,0,0,{n}]}}"
    )));
}

/// `LivingEntity.baseTick` counts `lastHurtByPlayerMemoryTime` down and drops
/// `lastHurtByPlayer` on the first tick whose time is already zero: the death
/// rewards (`dropAllDeathLoot`'s `playerKilled`, `dropExperience`) and
/// `getKillCredit` then find no player. A pig that burns 101 ticks after the
/// hit is nobody's kill, while the same death right after the hit is the
/// player's.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_player_memory_that_ran_out_credits_nobody() {
    let mut game = game();
    // A hit just landed: the lava death is the player's kill.
    summon(&mut game, "pig", "4.5 100 4.5", 1, 10.0);
    let pig = index(&mut game, PIG_A);
    hit_by_player(&mut game, PIG_A, 1.0);
    assert_eq!(
        game.mobs[pig].ai.last_hurt_by_player, 100,
        "the hit is remembered"
    );
    assert!(game.run_command(&format!("damage {PIG_A} 20 minecraft:lava")));
    step(&mut game, 1);
    assert_eq!(pigs_killed(&game), 1, "a fresh hit is the player's kill");
    let orbs = game.orbs.len();
    assert!(orbs > 0, "a player kill drops experience");
    // 101 ticks on, the memory has run out and `baseTick` dropped the player
    // on the 101st: the same lava death credits nobody.
    summon(&mut game, "pig", "4.5 100 4.5", 2, 10.0);
    hit_by_player(&mut game, PIG_B, 1.0);
    step(&mut game, 101);
    let pig = index(&mut game, PIG_B);
    assert_eq!(
        game.mobs[pig].ai.last_hurt_by_player, 0,
        "the memory ran out"
    );
    assert!(game.run_command(&format!("damage {PIG_B} 20 minecraft:lava")));
    step(&mut game, 1);
    assert_eq!(
        pigs_killed(&game),
        1,
        "a hit 101 ticks old is nobody's kill"
    );
    assert_eq!(game.orbs.len(), orbs, "no experience dropped");
}

/// `Raider.die`'s hero of the village and `Villager.die`'s murderer are the
/// killing blow's causing entity (`DamageSource.getEntity`), not the player
/// `getKillCredit` names: a villager that burns to death, even just after a
/// player hit it, was not murdered by that player.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_death_without_a_causing_entity_is_no_murder() {
    let mut game = game();
    summon(&mut game, "villager", "4.5 100 4.5", 3, 20.0);
    summon(&mut game, "villager", "6.5 100 4.5", 4, 20.0);
    let victim = index(&mut game, VICTIM);
    let witness_at = index(&mut game, WITNESS);
    let witness = game.mobs[witness_at].id;
    let_sees_witness(&mut game, victim, witness);
    // Hit just now, then lava: the killing blow has no causing entity, so no
    // murder, whatever player the hurt memory names.
    hit_by_player(&mut game, VICTIM, 1.0);
    assert!(game.run_command(&format!("damage {VICTIM} 20 minecraft:lava")));
    step(&mut game, 1);
    assert_eq!(
        game.villager_reputation_of(witness, PLAYER),
        0,
        "a lava death is no murder"
    );
    // The control: the same villager, killed by a player's blow, is murdered.
    summon(&mut game, "villager", "5.5 100 5.5", 5, 20.0);
    let murdered = index(&mut game, MURDERED);
    let_sees_witness(&mut game, murdered, witness);
    hit_by_player(&mut game, MURDERED, 20.0);
    step(&mut game, 1);
    assert!(
        game.villager_reputation_of(witness, PLAYER) < 0,
        "a player's blow is a murder"
    );
}
