//! Actual server gameplay paths, paired with VanillaCrystalExplosionCauseOracle.
use super::*;
use crate::game::{boats::Boat, commands::Target, dragon::Crystal};
use horizon_client::{
    entities::ArmorStand,
    items::ItemStack,
    mobs::Mob,
    motion::PlayerMotion,
    projectiles::{Hit as ProjectileHit, Projectile, ProjectileKind},
    text::Text,
};
use horizon_world::nbt::Tag;

fn name(game: &mut Game<'_>, id: u32, name: &str) {
    let tag = Tag::compound([("CustomName", Tag::String(name.into()))]);
    game.persist.raw.insert(id, tag);
}

#[test]
#[ignore = "requires prepared resources; tools/test_crystal_explosion_causes_parity.py"]
fn crystal_explosion_causes_parity_fixture() {
    run(
        &[
            "crystal",
            "crystal_snowball",
            "crystal_arrow",
            "crystal_named",
        ],
        16,
    );
}

fn run(modes: &[&str], expected: usize) {
    let mut output = Vec::new();
    let mut game = crate::game::native_machines::tests::server_game();
    game.build_registries();
    game.next_entity = 100;
    for &mode in modes {
        for owner in ["player", "guest", "mob", "none"] {
            game.combat = Default::default();
            game.dropped.clear();
            game.orbs.clear();
            game.mobs.clear();
            game.tnts.clear();
            game.armor_stands.clear();
            game.creatures.projectiles.clear();
            game.persist.raw.clear();
            game.last_hurt_by_player.clear();
            game.inventory.fill(None);
            game.lan_acting = (owner == "guest").then_some(11);
            name(&mut game, 7, "Blast");
            game.creative = false;
            game.ui.difficulty = 2;
            assert!(game.ui.rules.set_string("mob_griefing", "false"));
            game.boats
                .push(Boat::new(45, "oak".into(), Vec3::new(6.5, 64.0, 8.5), 0.0));
            game.armor_stands
                .push(ArmorStand::new(46, Vec3::new(8.5, 64.0, 6.5), 0.0));
            let registry = &game.pack.assets.registry;
            let tnt = registry.default_state("minecraft:tnt").unwrap();
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
                sword.data_mut().custom_name = Some(Tag::String("Sword".into()));
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
            let shooter = match owner {
                "player" | "guest" => Some(Who::Player),
                "mob" => Some(Who::Mob(43)),
                _ => None,
            };
            let guest = (owner == "guest").then_some(11);
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
                let mut projectile = Projectile::new(kind, Vec3::new(8.5, 64.0, 8.5), shooter);
                projectile.id = 71;
                projectile.guest = guest;
                projectile.velocity.x = 3.0;
                name(&mut game, 71, "Projectile");
                game.projectile_hit(
                    &projectile,
                    ProjectileHit::Entity {
                        who: Who::Mob(crate::game::dragon::CRYSTAL_IDS),
                        at: projectile.position,
                    },
                )
                .unwrap();
            }
            assert!(game.crystals.is_empty());
            assert!(game.player.as_ref().unwrap().health.dead_or_dying());
            assert!(
                game.boats.is_empty(),
                "a causing mob does not turn the crystal into a mob explosion"
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
            assert_eq!(credit, if id == 41 || id == 43 { id } else { 0 });
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
            let death = game.combat_death_message_for(Text::literal("Victim"));
            let component = death.component();
            let named = component["with"][1]["text"].as_str().unwrap_or("-");
            output.push(format!(
                "{mode}/{owner} {damage} {id} {credit} {} {named}",
                death.translation_key().unwrap()
            ));
            assert!(game.combat.explicit_explosion_cause.is_none());
        }
    }
    assert_eq!(output.len(), expected);
    std::fs::write(
        std::env::var_os("HORIZON_CRYSTAL_CAUSES_OUTPUT").expect("fixture output"),
        output.join("\n") + "\n",
    )
    .unwrap();
}
