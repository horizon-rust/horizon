use super::*;
use crate::game::native_entity_refs::tests::with_game;
use mod_api::{Hooks, ModEntry, PlayerMilkEntityEvent, ServerHost};
use std::sync::{Arc, Mutex};

type Heard = Arc<Mutex<Vec<(u128, String, i32, String)>>>;

struct MilkProbe(Heard);
#[mod_api::__subscriptions]
impl Hooks for MilkProbe {
    fn player_milk_entity(&self, _: &mut dyn ServerHost, event: &mut PlayerMilkEntityEvent<'_>) {
        self.0.lock().unwrap().push((
            event.player,
            event.name.to_owned(),
            event.entity_id,
            event.entity.to_owned(),
        ));
    }
}

/// A bucket on a cow and on a goat is heard once each, with the species; a bucket on a calf and
/// a bowl on a mooshroom are not milking and are not heard.
#[test]
#[ignore = "daily"]
fn milking_with_a_bucket_is_heard_with_the_species() {
    use horizon_client::motion::PlayerMotion;
    let heard = Heard::default();
    let mods: mod_api::Mods = vec![ModEntry::new("milk-probe", Box::new(MilkProbe(heard.clone())))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 0.5)).unwrap());
            game.lan_acting = Some(12);
            game.lan_acting_uuid = Some(7);
            crate::game::native_entity_refs::tests::bind_local_player(game, 2, 7);
            game.lan_acting_name = Some("Milker".into());
            game.lan_acting_permission = Some(4);
            game.lan_acting_entity = Some(2);
            crate::game::native_entity_refs::tests::bind_acting_guest(game);
            game.selected = 0;
            game.set_mode_id(0);
            let milk = |game: &mut Game<'_>, kind: &str, item: &str| {
                game.mobs.clear();
                let kind = game.res.mob_types().get(kind).unwrap().clone();
                game.mobs
                    .push(Mob::new(31, kind, Vec3::new(2.5, 64.0, 2.5), 0.0));
                game.inventory[0] = Some(ItemStack::new(item, 1));
                let target = Some(InteractTarget {
                    id: 31,
                    location: [0.0, 0.0, 0.0],
                });
                game.interact_mob(target).unwrap();
            };
            milk(game, "minecraft:cow", "minecraft:bucket");
            milk(game, "minecraft:goat", "minecraft:bucket");
            milk(game, "minecraft:mooshroom", "minecraft:bowl");
            let expected = [
                (7, "Milker".to_owned(), 31, "minecraft:cow".to_owned()),
                (7, "Milker".to_owned(), 31, "minecraft:goat".to_owned()),
            ];
            assert_eq!(*heard.lock().unwrap(), expected);
        });
    });
}
