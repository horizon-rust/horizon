//! `AbstractMinecart`'s rider seat and dismount side: a villager or wandering
//! trader sits at the cart's own y, and the old behavior sets a rider down
//! beside its motion direction, not beside its facing.
use super::super::mounts::DismountWorld;
use super::*;

/// `AbstractMinecart.getPassengerAttachmentPoint` lowers a villager's and a
/// wandering trader's seat to the cart's own y; any other rider keeps the
/// cart's 0.1875 attachment.
#[test]
fn a_villager_sits_on_the_cart_itself() {
    let at = Vec3::new(1.0, 10.0, 2.0);
    for species in [Species::Villager, Species::WanderingTrader] {
        assert_eq!(cart_attachment(species), 0.0, "{species:?}");
        // Neither has a `ridingOffset` of its own.
        let seat = cart_seat(at, cart_seat_offset(species, 0.0));
        assert!(
            (seat.y - 10.0).abs() < 1e-12,
            "{species:?} sits at {seat:?}"
        );
    }
    // A cow keeps the cart's 0.1875, less a `ridingOffset` if it had one.
    let cow = cart_seat(at, cart_seat_offset(Species::Cow, 0.0));
    assert!((cow.y - 10.1875).abs() < 1e-12, "a cow sits at {cow:?}");
    assert_eq!(cart_seat_offset(Species::Cow, 0.7), PASSENGER_Y - 0.7);
    // A zombie villager is no `Villager`.
    assert_eq!(cart_attachment(Species::ZombieVillager), PASSENGER_Y);
}

/// `AbstractMinecart.getDismountLocationForPassenger` starts the search from
/// `getMotionDirection`, and `DismountHelper.offsetsForDirection` tries the
/// right of that direction first.
#[test]
fn an_eastbound_cart_sets_its_rider_down_on_its_south_side() {
    // `OldMinecartBehavior.tick`'s yaw of a cart moving east, flipped or not;
    // its `getMotionDirection` turns that facing clockwise, to east.
    for (yaw, flipped) in [(180.0, false), (0.0, true)] {
        let face = cart_motion_face(yaw, flipped, false);
        assert_eq!(
            dismount_offsets(face)[0],
            (0, 1),
            "yaw {yaw}, flipped {flipped}"
        );
    }
    // `MinecartBehavior.getMotionDirection` under the experiment is the
    // facing itself, so the first candidate stays east of a north-facing cart.
    let face = cart_motion_face(180.0, false, true);
    assert_eq!(dismount_offsets(face)[0], (1, 0));
}

/// Stone at y 63, a soul sand block in its place on the floor at `soul`, and
/// walls at the blocks listed.
struct Chamber {
    soul: [i32; 3],
    walls: Vec<[i32; 3]>,
}
impl Chamber {
    fn solid(&self, p: [i32; 3]) -> bool {
        p[1] == 63 || self.walls.contains(&p)
    }
}
impl DismountWorld for Chamber {
    fn lava(&self, _: [i32; 3]) -> bool {
        false
    }
    /// `BlockGetter.getBlockFloorHeight`: soul sand's top is 14/16.
    fn floor_height(&self, p: [i32; 3]) -> f64 {
        if p == self.soul {
            0.875
        } else if self.solid(p) {
            1.0
        } else if self.solid([p[0], p[1] - 1, p[2]]) {
            0.0
        } else {
            f64::NEG_INFINITY
        }
    }
    fn can_dismount_to(&self, bounds: Aabb) -> bool {
        let (lo, hi) = (bounds.min_position(), bounds.max_position());
        let cells = |a: f64, b: f64| a.floor() as i32..=(b - 1.0e-7).floor() as i32;
        let blocked =
            |p: [i32; 3]| self.solid(p) || (p == self.soul && lo.y < f64::from(p[1]) + 0.875);
        !cells(lo.x, hi.x)
            .any(|x| cells(lo.y, hi.y).any(|y| cells(lo.z, hi.z).any(|z| blocked([x, y, z]))))
    }
}

/// `AbstractMinecart.getDismountLocationForPassenger` for a mob (#16907): a
/// shulker leaving an eastbound cart lands on the soul sand south of the
/// track, the first side with room, at the sand's 0.875 top; with that side
/// walled in, on the north side instead, never on the cart's own block.
#[test]
fn a_mob_leaves_a_cart_beside_the_track() {
    let cart = Vec3::new(0.5, 64.0, 0.5);
    let face = cart_motion_face(180.0, false, false);
    let open = Chamber {
        soul: [0, 64, 1],
        walls: vec![],
    };
    let at = mob_dismount_location(cart, face, 1.0, 1.0, &open);
    assert_eq!(at, Vec3::new(0.5, 64.875, 1.5));
    let walled = Chamber {
        soul: [0, 64, 1],
        walls: vec![[0, 65, 1]],
    };
    let at = mob_dismount_location(cart, face, 1.0, 1.0, &walled);
    assert_eq!(at, Vec3::new(0.5, 64.0, -0.5));
}

/// A new shulker riding a rideable cart at (8.5, 100, 8.5), in an air box with
/// `blocks` set, and the cart's id.
fn shulker_in_cart(game: &mut Game<'_>, blocks: &[([i32; 3], &str)]) -> u32 {
    game.build_registries();
    game.set_allow_commands(true);
    game.player = Some(
        horizon_client::motion::PlayerMotion::new(game.motion, Vec3::new(8.5, 100.0, 8.5)).unwrap(),
    );
    game.update_chunk_tickets();
    let registry = game.pack.assets.registry;
    let air = registry.default_state("minecraft:air").unwrap();
    for x in 6..=10 {
        for y in 98..=103 {
            for z in 6..=10 {
                game.world.set(x, y, z, air).unwrap();
            }
        }
    }
    for &([x, y, z], name) in blocks {
        game.world
            .set(x, y, z, registry.default_state(name).unwrap())
            .unwrap();
    }
    assert!(game.run_command("summon minecraft:shulker 8.5 100 8.5 {NoAI:1b}"));
    let id = 900_000;
    game.carts.clear();
    game.carts.push(Minecart::new(
        id,
        CartKind::Rideable,
        Vec3::new(8.5, 100.0, 8.5),
    ));
    let m = game
        .mobs
        .iter_mut()
        .rfind(|m| m.species() == Species::Shulker)
        .expect("the shulker");
    m.extra.mount = Some(id);
    m.extra.passenger = true;
    id
}

/// The last shulker summoned: where it is and what it rides.
fn shulker_at(game: &Game<'_>) -> (Vec3, Option<u32>) {
    let m = game
        .mobs
        .iter()
        .rfind(|m| m.species() == Species::Shulker)
        .unwrap();
    (m.position, m.extra.mount)
}

/// `Minecart.activateMinecart` → `ejectPassengers` → `dismountVehicle`
/// (#16907): a powered activator rail sets its shulker down where the cart
/// finds room, on the soul sand beside it, and `Shulker.setPos` lifts it from
/// the sand's 0.875 top into the block above; a ladder is no floor there
/// (`DismountHelper.nonClimbableShape`); and a cart in a portal lets it off
/// where it is.
#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_activator_rail_sets_a_shulker_down_beside_the_cart() {
    let mut game = super::super::native_machines::tests::server_game();

    shulker_in_cart(&mut game, &[([9, 100, 8], "minecraft:soul_sand")]);
    let mut cart = game.carts[0].clone();
    game.activate_cart(&mut cart, true);
    assert_eq!(shulker_at(&game), (Vec3::new(9.5, 101.0, 8.5), None));

    // A ladder's top is no floor for the block above it: with nowhere else,
    // the shulker gets off on top of the cart (100.7), snapped to 101.
    shulker_in_cart(&mut game, &[([9, 100, 8], "minecraft:ladder")]);
    let mut cart = game.carts[0].clone();
    game.activate_cart(&mut cart, true);
    assert_eq!(shulker_at(&game), (Vec3::new(8.5, 101.0, 8.5), None));

    shulker_in_cart(
        &mut game,
        &[
            ([8, 100, 8], "minecraft:nether_portal"),
            ([9, 100, 8], "minecraft:soul_sand"),
        ],
    );
    let mut cart = game.carts[0].clone();
    game.activate_cart(&mut cart, true);
    assert_eq!(shulker_at(&game), (Vec3::new(8.5, 100.0, 8.5), None));
}
