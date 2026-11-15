//! `Mob.doHurtTarget` takes `getAttributeValue(ATTACK_DAMAGE)` as it is — vanilla floors no
//! damage up to 1 — and `LivingEntity.actuallyHurt`'s `if (dmg != 0.0F)` skips `setHealth` and
//! `gameEvent(GameEvent.ENTITY_DAMAGE)` for a blow that leaves nothing. The hurt sound and the
//! knockback of `LivingEntity.hurtServer` still happen: the blow landed, it just took nothing.
use super::*;
use crate::game::*;
use horizon_client::mobs::{Mob, Who, ai::do_hurt_target, goals::Cx};
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, LevelRandom},
};

/// A windowless integrated-server game over the prepared resources, or `None` without them.
fn with_game<R>(f: impl FnOnce(&mut Game<'_>) -> R) -> Option<R> {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES")?);
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        Instant::now(),
        false,
        true,
        &Progress::new(),
        &mut |_, _| unreachable!(),
    )
    .ok()?;
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = execution_role::ExecutionRole::IntegratedServer;
    let mut g = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    g.options.dedicated = Some(Vec::new());
    g.world = World::new(g.res.facts().air, 1);
    Some(f(&mut g))
}

/// What one zombie blow of `attack_damage` left: the pig's health, the sounds the level played
/// and the game events posted (`dispense_fixture`'s `G x,y,z event` lines, the end-of-tick scan
/// included).
fn hit_with(g: &mut Game<'_>, attack_damage: f32) -> (f32, Vec<String>, Vec<String>) {
    g.mobs.clear();
    g.audio.capture = Some(Vec::new());
    for (id, kind) in [(41, "minecraft:zombie"), (42, "minecraft:pig")] {
        let kind = g.res.mob_types().get(kind).unwrap().clone();
        let mut m = Mob::new(id, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        m.health = 20.0;
        g.mobs.push(m);
    }
    // `/attribute … minecraft:attack_damage base set 0`: the blow carries no damage.
    g.mobs[0].attrs.attack_damage = attack_damage;
    let mut mobs = std::mem::take(&mut g.mobs);
    let mut lr = LevelRandom::for_level(LegacyRandom::new(0));
    let mut events = Vec::new();
    {
        let w = g.mob_world_base(&[]);
        let mut cx = Cx {
            w: &w,
            lr: &mut lr,
            events: &mut events,
        };
        do_hurt_target(&mut mobs[0], Who::Mob(42), &mut cx);
    }
    g.mobs = mobs;
    crate::game::dispense_fixture::start_recording();
    g.handle_mob_events(events).unwrap();
    // `actuallyHurt`'s damage vibration reaches the scan at the tick's end, not from the blow.
    g.world.game_time += 1;
    g.tick_mob_sculk_events();
    let posted = crate::game::dispense_fixture::stop_recording();
    let sounds = g
        .audio
        .capture
        .take()
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.event)
        .collect();
    (g.mobs[1].health, sounds, posted)
}

/// A zombie whose `attack_damage` is 0 leaves its target's health alone and posts no
/// `ENTITY_DAMAGE`; the blow still lands, so the victim cries out. At 1 it takes 1 and posts
/// the vibration.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_blow_with_no_attack_damage_takes_nothing_and_posts_no_damage_event() {
    let Some(outcome) = with_game(|g| {
        let nothing = hit_with(g, 0.0);
        let one = hit_with(g, 1.0);
        (nothing, one)
    }) else {
        return;
    };
    let (health, sounds, posted) = outcome.0;
    assert_eq!(health, 20.0, "no damage reaches the health: {sounds:?}");
    assert!(
        sounds.iter().any(|s| s.ends_with(".hurt")),
        "the blow still landed, so the victim cried out: {sounds:?}"
    );
    assert!(
        !posted
            .iter()
            .any(|l| l.ends_with("minecraft:entity_damage")),
        "a hit that takes nothing posts no damage vibration: {posted:?}"
    );
    let (health, _, posted) = outcome.1;
    assert_eq!(health, 19.0, "a one-damage blow takes one");
    assert!(
        posted
            .iter()
            .any(|l| l.ends_with("minecraft:entity_damage")),
        "a blow that takes something posts its damage vibration: {posted:?}"
    );
}
