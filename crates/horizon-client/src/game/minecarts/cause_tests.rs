//! Actual server gameplay paths, paired with VanillaExplosionCausesOracle.
use super::*;
use crate::game::{commands::Target, dragon::Crystal};
use horizon_client::{
    mobs::Mob,
    motion::PlayerMotion,
    projectiles::{Hit as ProjectileHit, Projectile, ProjectileKind},
    text::Text,
};

fn shot(kind: ProjectileKind, owner: Option<Who>, guest: Option<u32>) -> Projectile {
    let mut projectile = Projectile::new(kind, Vec3::new(8.5, 64.0, 8.5), owner);
    projectile.id = 71;
    projectile.guest = guest;
    projectile.velocity.x = 3.0;
    projectile
}

fn hit(game: &mut Game<'_>, projectile: &Projectile, id: u32) {
    game.projectile_hit(
        projectile,
        ProjectileHit::Entity {
            who: Who::Mob(id),
            at: projectile.position,
        },
    )
    .unwrap();
}

#[test]
#[ignore = "requires prepared resources; tools/test_explosion_causes_parity.py"]
fn explosion_causing_entities_parity_fixture() {
    let mut output = Vec::new();
    let mut game = crate::game::native_machines::tests::server_game();
    game.next_entity = 100;
    for mode in [
        "crystal",
        "crystal_snowball",
        "crystal_arrow",
        "crystal_named",
        "cart",
        "cart_named",
        "cart_command",
        "cart_removed",
        "cart_reloaded",
    ] {
        for owner in ["player", "guest", "mob", "none"] {
            game.combat = Default::default();
            game.dropped.clear();
            game.orbs.clear();
            game.mobs.clear();
            game.carts.clear();
            game.tnts.clear();
            game.boats.clear();
            game.armor_stands.clear();
            game.crystals.clear();
            game.creatures.projectiles.clear();
            game.persist.raw.clear();
            game.last_hurt_by_player.clear();
            game.inventory.fill(None);
            game.lan_acting = (owner == "guest").then_some(11);
            game.persist.raw.insert(
                7,
                horizon_world::nbt::Tag::compound([(
                    "CustomName",
                    horizon_world::nbt::Tag::String("Blast".into()),
                )]),
            );
            game.creative = false;
            game.ui.difficulty = 2;
            assert!(game.ui.rules.set_string("mob_griefing", "false"));
            game.boats.push(crate::game::boats::Boat::new(
                45,
                "oak".into(),
                Vec3::new(6.5, 64.0, 8.5),
                0.0,
            ));
            game.armor_stands
                .push(horizon_client::entities::ArmorStand::new(
                    46,
                    Vec3::new(8.5, 64.0, 6.5),
                    0.0,
                ));
            let tnt = game
                .pack
                .assets
                .registry
                .default_state("minecraft:tnt")
                .unwrap();
            game.world.set(9, 64, 9, tnt).unwrap();
            game.lan_acting_name = Some("Igniter".into());
            game.lan_acting_entity = Some(41);
            let mut player = PlayerMotion::new(game.motion, Vec3::new(10.5, 64.0, 8.5)).unwrap();
            player.entity_id = 42;
            game.player = Some(player);
            let kind = game.res.mob_types().get("minecraft:husk").unwrap().clone();
            game.mobs
                .push(Mob::new(44, kind.clone(), Vec3::new(8.5, 64.0, 10.5), 0.0));
            let mut mob = Mob::new(43, kind, Vec3::new(15.5, 64.0, 15.5), 0.0);
            mob.custom_name = Some("Igniter".into());
            mob.custom_name_text = Some(Text::literal("Igniter"));
            if mode.ends_with("named") {
                let mut sword = ItemStack::new("minecraft:diamond_sword", 1);
                sword.data_mut().custom_name =
                    Some(horizon_world::nbt::Tag::String("Sword".into()));
                if owner == "mob" {
                    mob.extra.set_main_hand_stack(Some(sword));
                } else {
                    game.inventory[game.selected] = Some(sword);
                }
            }
            game.mobs.push(mob);
            let cause = match owner {
                "player" | "guest" => Some(Target::Player),
                "mob" => Some(Target::Mob(43)),
                _ => None,
            };
            let shooter = cause.map(|t| {
                if t == Target::Player {
                    Who::Player
                } else {
                    Who::Mob(43)
                }
            });
            let guest = (owner == "guest").then_some(11);
            let arrow_kind = ProjectileKind::Arrow {
                base_damage: 2.0,
                critical: false,
                effect: None,
            };
            if mode.starts_with("crystal") {
                game.crystals.push(Crystal {
                    id: 7,
                    position: Vec3::new(8.5, 64.0, 8.5),
                    show_bottom: true,
                    time: 0,
                    beam: None,
                    invulnerable: false,
                });
                if mode == "crystal" || mode == "crystal_named" {
                    assert!(game.command_damage_crystal(
                        7,
                        "minecraft:generic",
                        horizon_client::mobs::DamageKind::Generic,
                        cause
                    ));
                } else {
                    let kind = if mode == "crystal_snowball" {
                        ProjectileKind::Snowball
                    } else {
                        ProjectileKind::Arrow {
                            base_damage: 2.0,
                            critical: false,
                            effect: None,
                        }
                    };
                    let projectile = shot(kind, shooter, guest);
                    game.persist.raw.insert(
                        71,
                        horizon_world::nbt::Tag::compound([(
                            "CustomName",
                            horizon_world::nbt::Tag::String("Projectile".into()),
                        )]),
                    );
                    hit(&mut game, &projectile, crate::game::dragon::CRYSTAL_IDS);
                }
                assert!(game.crystals.is_empty());
            } else {
                let mut cart = Minecart::new(7, CartKind::Tnt, Vec3::new(8.5, 64.0, 8.5));
                cart.no_gravity = true;
                cart.velocity.x = 0.2;
                game.carts.push(cart);
                if mode == "cart_command" {
                    let by = match owner {
                        "player" | "guest" => " by @s",
                        "mob" => " by @e[type=husk,name=Igniter,limit=1]",
                        _ => "",
                    };
                    assert!(game.run_command(&format!(
                        "damage @e[type=tnt_minecart,limit=1] 1 in_fire{by}"
                    )));
                } else if cause.is_some() {
                    let arrow = shot(arrow_kind.clone(), shooter, guest);
                    hit(&mut game, &arrow, 7);
                } else {
                    game.hurt_cart(0, 1.0, false, true);
                }
                // Even a live NBT fuse reset cannot replace the first ignition source.
                let mut reset = game.carts[0].to_nbt([1, 2, 3, 4], None);
                if let horizon_world::nbt::Tag::Compound(tag) = &mut reset {
                    tag.insert("fuse".into(), horizon_world::nbt::Tag::Int(-1));
                }
                game.carts[0].load(&reset).unwrap();
                let later = shot(arrow_kind, Some(Who::Mob(43)), None);
                hit(&mut game, &later, 7);
                assert!(game.carts[0].fuse >= 0, "the attack primed the moving cart");
                game.carts[0].fuse = 2;
                game.carts[0].velocity = Vec3::ZERO;
                if mode == "cart_removed" {
                    game.mobs.retain(|m| m.id != 43);
                    game.lan_acting_entity = None;
                    game.lan_acting_name = Some("Victim".into());
                }
                if mode == "cart_reloaded" {
                    let saved = game.carts[0].to_nbt([1, 2, 3, 4], None);
                    game.carts[0] = Minecart::from_nbt(&saved, 7, CartKind::Tnt).unwrap();
                }
                for tick in 0..3 {
                    let mut cart = game.carts[0].clone();
                    let alive = game.tick_cart(&mut cart, [0.0; 2]);
                    if tick < 2 {
                        assert!(alive);
                        assert!(!game.player.as_ref().unwrap().health.dead_or_dying());
                        game.carts[0] = cart;
                    } else {
                        assert!(!alive);
                    }
                }
                assert!(game.carts.is_empty());
            }
            assert!(game.player.as_ref().unwrap().health.dead_or_dying());
            assert!(
                game.boats.is_empty(),
                "a causing mob does not turn the cart/crystal into a mob explosion"
            );
            assert!(!game.tnts.is_empty());
            assert!(
                game.tnts
                    .iter()
                    .all(|t| t.owner.is_none() && t.guest.is_none()),
                "TntBlock.wasExploded uses the explosion's indirect source, not its explicit damage cause"
            );
            let events = std::mem::take(&mut game.player.as_mut().unwrap().events);
            game.record_combat(&events);
            let (damage, id) = game.combat.last_blow().unwrap();
            let damage = damage.to_owned();
            let credit = game.combat.kill_credit(game.world.game_time).unwrap_or(0);
            assert_eq!(
                credit,
                if mode != "cart_removed" && (id == 41 || id == 43) {
                    id
                } else {
                    0
                }
            );
            assert_eq!(
                game.armor_stands.iter().any(|a| a.id == 46),
                id == 43,
                "decoration hurt reads the explicit damage cause, unlike vehicle ignoreExplosion"
            );
            let mob = game.mobs.iter().find(|m| m.id == 44).unwrap();
            assert!(!mob.alive());
            assert_eq!(
                mob.ai.last_hurt_by,
                match id {
                    41 => Some(Who::Player),
                    43 => Some(Who::Mob(43)),
                    _ => None,
                }
            );
            assert_eq!(
                game.last_hurt_by_player.get(&44).copied(),
                (id == 41).then_some((owner == "guest").then_some(11))
            );
            if mode == "cart_removed" && owner == "mob" {
                assert!(
                    !game.combat.exploded_kinds.iter().any(|(id, _)| *id == 43),
                    "a removed ignition owner is not an early-removed current exploder"
                );
            }
            let death = game.combat_death_message_for(Text::literal("Victim"));
            let component = death.component();
            let named = component
                .get("with")
                .and_then(|v| v.get(1))
                .and_then(|v| v.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("-");
            output.push(format!(
                "{mode}/{owner} {damage} {id} {credit} {} {named}",
                death.translation_key().unwrap()
            ));
            assert!(game.combat.explicit_explosion_cause.is_none());
        }
    }
    std::fs::write(
        std::env::var_os("HORIZON_EXPLOSION_CAUSES_OUTPUT").expect("fixture output"),
        output.join("\n") + "\n",
    )
    .unwrap();
}
