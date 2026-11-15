//! `Mob.swingForAttack(hand)`: an AI melee swing plays the held stack's
//! `attack_animation` (`ItemStack.getAttackAnimation`) for as long as
//! `getModifiedSwingDuration` says (haste, conduit power, mining fatigue), and
//! `SwingState.startIfAble` refuses a swing asked for inside the first half of
//! the current one, `/swing` included.
use crate::first_person::SwingAnimation;
use crate::items::ItemStack;
use crate::mobs::ai::tick_swing;
use crate::mobs::{Mob, Species};
use horizon_core::geometry::Vec3;
use std::sync::Arc;

/// The repro's zombie: right-handed, holding nothing yet.
fn zombie() -> Mob {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let kind_mut = Arc::make_mut(&mut kind);
    kind_mut.species = Species::Zombie;
    kind_mut.id = "minecraft:zombie".into();
    Mob::new(1, kind, Vec3::ZERO, 0.0)
}

/// The repro: a zombie holding an iron spear thrusts for the spear's 19 ticks
/// (`(int)(0.95F * 20.0F)`) as a STAB of the main arm; a bare-handed zombie
/// whacks for `SwingAnimation.DEFAULT`'s 6.
#[test]
fn a_melee_swing_plays_the_held_weapons_animation() {
    let mut m = zombie();
    m.extra
        .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_spear", 1)));
    m.swing_for_attack(false);
    assert!(m.ai.swinging, "the attack swings");
    assert_eq!(m.swing_duration(), 19, "the iron spear's attack animation");
    assert!(m.extra.swing_stab, "a STAB, not the default whack");
    let main_arm = !m.main_arm_left();
    assert_eq!(
        m.extra.swing_right_arm,
        Some(main_arm),
        "the main hand's arm"
    );

    let mut bare = zombie();
    bare.swing_for_attack(false);
    assert_eq!(bare.swing_duration(), 6, "SwingAnimation.DEFAULT");
    assert!(!bare.extra.swing_stab);
}

/// `getModifiedSwingDuration`: haste (and conduit power) shorten the swing,
/// mining fatigue lengthens it.
#[test]
fn haste_and_mining_fatigue_change_the_swing_length() {
    let spear = || {
        let mut m = zombie();
        m.extra
            .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_spear", 1)));
        m
    };
    let mut fast = spear();
    fast.extra.effects.push(("minecraft:haste", 200, 1));
    fast.swing_for_attack(false);
    assert_eq!(fast.swing_duration(), 17, "19 - (1 + haste's amplifier)");

    let mut slow = spear();
    slow.extra
        .effects
        .push(("minecraft:mining_fatigue", 200, 1));
    slow.swing_for_attack(false);
    assert_eq!(
        slow.swing_duration(),
        23,
        "19 + (1 + mining fatigue's amplifier) * 2"
    );
}

/// `SwingState.startIfAble`: `/swing` (`LivingEntity.swing(hand, animation,
/// true)`) in the first half of a spear's thrust leaves it alone, and past the
/// half restarts it with the command's own animation.
#[test]
fn a_swing_inside_the_first_half_is_refused() {
    let mut m = zombie();
    m.extra
        .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_spear", 1)));
    m.swing_for_attack(false);
    // One tick later: `SwingState.ticks` is 1, inside the first half of 19.
    tick_swing(&mut m);
    let ticks = m.ai.swing_time;
    m.swing_hand_modified(false, SwingAnimation::DEFAULT);
    assert_eq!(
        m.ai.swing_time, ticks,
        "the refused swing leaves the animation"
    );
    assert_eq!(m.swing_duration(), 19, "and the spear's length");
    assert!(m.extra.swing_stab);
    // `ticks <= durationTicks / 2` holds up to 9; the 10th tick is past it.
    while m.ai.swing_time < 9 {
        tick_swing(&mut m);
    }
    m.swing_hand_modified(false, SwingAnimation::DEFAULT);
    assert_eq!(m.ai.swing_time, -1, "past the first half it restarts");
    assert_eq!(m.swing_duration(), 6, "with the command's animation");
    assert!(!m.extra.swing_stab);
}

/// The wiring: `MeleeAttackGoal.checkAndPerformAttack` swings through
/// `swingForAttack(MAIN_HAND)`, so the zombie of the repro thrusts its spear
/// when it hits, and the hit still lands.
#[test]
fn a_zombies_melee_goal_swings_the_spear_it_holds() {
    use crate::mobs::goals::{Cx, GoalSelector, MeleeAttack};
    use crate::mobs::world::{test_player, with_test_world};
    use crate::mobs::{MobEvent, Who};

    let mut out = None;
    let ran = with_test_world(&[], |w, types| {
        let kind = Arc::new(types.types["minecraft:zombie"].clone());
        let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        m.extra
            .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_spear", 1)));
        m.ai.target = Some(Who::Player);
        let mut world = w.clone();
        // A spear's reach is 1.0 to 2.25 blocks for a mob (its `mob_factor`
        // halves `attack_range`), so the target stands 2.5 blocks away.
        world.player = Some(test_player(Vec3::new(0.5, 64.0, 2.5)));
        let mut goals = GoalSelector::default();
        goals.add(1, MeleeAttack::new(1.0, true));
        let (mut lr, mut events) = (crate::mobs::world::level_random(0), Vec::new());
        goals.tick(
            &mut m,
            &mut Cx {
                w: &world,
                lr: &mut lr,
                events: &mut events,
            },
        );
        let hit = events
            .iter()
            .any(|e| matches!(e, MobEvent::AttackPlayer(..)));
        out = Some((m.ai.swinging, m.swing_duration(), m.extra.swing_stab, hit));
    });
    if !ran {
        return;
    }
    assert_eq!(
        out,
        Some((true, 19, true, true)),
        "the goal's attack swings the spear for its 19 ticks and still hits"
    );
}
