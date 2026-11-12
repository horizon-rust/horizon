//! `EndGatewayBlock.entityInside` for the entity kinds beyond the items and
//! mobs: a gateway sends arrows, other projectiles, XP orbs, primed TNT and
//! falling blocks to its exit with their motion kept.

use super::super::native_machines::tests::server_game;
use super::super::{Dim, FallingBlock, Game};
use horizon_client::chunk::Chunk;
use horizon_client::containers::{BlockEntity, EndGateway};
use horizon_client::mobs::Mob;
use horizon_client::projectiles::{Projectile, ProjectileKind};
use horizon_core::geometry::Vec3;
use horizon_core::position::BlockPos;
use std::sync::Arc;

const GATEWAY: [i32; 3] = [40, 64, 8];
const EXIT: [i32; 3] = [80, 64, 8];

/// The End with a fresh gateway at [`GATEWAY`] whose exit is [`EXIT`].
fn world() -> Game<'static> {
    let mut game = server_game();
    game.dim = Dim::End;
    game.settings.simulation_distance = 4;
    game.settings.render_distance = 12;
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 65.0, 8.5)).unwrap(),
    );
    for c in [(2, 0), (5, 0)] {
        game.world
            .chunks
            .insert(c, Arc::new(Chunk::empty(c.0, c.1)));
    }
    let state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:end_gateway")
        .unwrap();
    game.world
        .set(GATEWAY[0], GATEWAY[1], GATEWAY[2], state)
        .unwrap();
    game.world.block_entities.insert(
        (GATEWAY[0], GATEWAY[1], GATEWAY[2]),
        BlockEntity::EndGateway(EndGateway {
            age: 300,
            exit_portal: Some(EXIT),
            exact_teleport: true,
        }),
    );
    game
}

/// Gateway discovery (an arrow or a falling block goes through in that pass),
/// next-tick handling and arrival, without ticking entity motion.
fn through(game: &mut Game<'_>) {
    game.find_end_gateway_portals();
    game.handle_end_gateway_portals();
    game.resolve_end_gateway_arrivals().unwrap();
}

fn exit() -> Vec3 {
    Vec3::new(
        f64::from(EXIT[0]) + 0.5,
        f64::from(EXIT[1]),
        f64::from(EXIT[2]) + 0.5,
    )
}

fn assert_moved(position: Vec3, velocity: Vec3, motion: Vec3) {
    assert_eq!(position, exit());
    assert_eq!(velocity, motion);
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_arrow_through_an_end_gateway_keeps_its_motion() {
    let mut game = world();
    let at = Vec3::new(40.5, 64.3, 8.5);
    let mut arrow = Projectile::new(
        ProjectileKind::Arrow {
            base_damage: 2.0,
            critical: false,
            effect: None,
        },
        at,
        None,
    );
    arrow.velocity = Vec3::new(1.5, 0.1, 0.0);
    game.add_projectile(arrow);
    let id = game.creatures.projectiles.last().unwrap().id;
    through(&mut game);
    let arrow = game
        .creatures
        .projectiles
        .iter()
        .find(|p| p.id == id)
        .expect("the arrow is still in the level");
    assert_moved(arrow.position, arrow.velocity, Vec3::new(1.5, 0.1, 0.0));
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_experience_orb_through_an_end_gateway_keeps_its_motion() {
    let mut game = world();
    game.add_orb([40.5, 64.2, 8.5], 1);
    let orb = game.orbs.last_mut().unwrap();
    orb.velocity = Vec3::new(0.1, 0.2, 0.0);
    let (id, motion) = (orb.id, orb.velocity);
    through(&mut game);
    let orb = game
        .orbs
        .iter()
        .find(|o| o.id == id)
        .expect("the orb is still in the level");
    assert_moved(orb.position, orb.velocity, motion);
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn primed_tnt_through_an_end_gateway_keeps_its_motion() {
    let mut game = world();
    game.add_primed_tnt(
        BlockPos::new(GATEWAY[0], GATEWAY[1], GATEWAY[2]),
        0.0,
        80,
        None,
        None,
    );
    let id = game.next_entity;
    game.tnts.iter_mut().find(|t| t.id == id).unwrap().velocity = Vec3::new(0.2, 0.1, 0.0);
    through(&mut game);
    let tnt = game
        .tnts
        .iter()
        .find(|t| t.id == id)
        .expect("the TNT is still in the level");
    assert_moved(tnt.position, tnt.velocity, Vec3::new(0.2, 0.1, 0.0));
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_falling_block_through_an_end_gateway_keeps_its_motion() {
    let mut game = world();
    let state = game
        .pack
        .assets
        .registry
        .default_state("minecraft:sand")
        .unwrap();
    let at = Vec3::new(40.5, 64.2, 8.5);
    let damage = horizon_client::falling_block::Damage::falling(&game.res.facts().get(state).name);
    game.next_entity += 1;
    let id = game.next_entity;
    game.falling.push(FallingBlock {
        rotation: [0.0, 0.0],
        join: Default::default(),
        id,
        state,
        start_pos: BlockPos::new(GATEWAY[0], GATEWAY[1], GATEWAY[2]),
        position: at,
        previous: at,
        velocity: Vec3::new(0.05, -0.04, 0.0),
        age: 0,
        no_gravity: true,
        portal: Default::default(),
        drop_item: true,
        fall_distance: 0.0,
        on_ground: false,
        stuck: Vec3::ZERO,
        damage,
        saved: None,
    });
    through(&mut game);
    let falling = game
        .falling
        .iter()
        .find(|f| f.id == id)
        .expect("the falling block is still in the level");
    assert_moved(
        falling.position,
        falling.velocity,
        Vec3::new(0.05, -0.04, 0.0),
    );
}

/// The control: a mob already travelled before the fix; it keeps its motion.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn a_moving_mob_through_an_end_gateway_keeps_its_motion() {
    let mut game = world();
    let kind = game
        .res
        .mob_types()
        .get("minecraft:zombie")
        .unwrap()
        .clone();
    game.next_entity += 1;
    let id = game.next_entity;
    let at = Vec3::new(40.5, 64.0, 8.5);
    let mut mob = Mob::new(id, kind, at, 0.0);
    mob.velocity = Vec3::new(0.1, 0.0, 0.0);
    game.mobs.push(mob);
    through(&mut game);
    let mob = game
        .mobs
        .iter()
        .find(|m| m.id == id)
        .expect("the mob is still in the level");
    assert_moved(mob.position, mob.velocity, Vec3::new(0.1, 0.0, 0.0));
}
