//! A mob discarded during the tick's packets leaves the level at once, as `Entity.discard`
//! does in vanilla: an interaction later in the same tick finds nothing, though the creature
//! pass only drops its entry at the end of the tick.

use horizon_client::{items::ItemStack, mobs::Mob};
use horizon_core::geometry::Vec3;
use horizon_protocol::packets::play::{Interact, Serverbound};
use mod_api::ActionRefused;

fn held(game: &crate::game::Game<'_>, conn: u32) -> Option<String> {
    let guest = game.lan.as_ref().unwrap().hub.guest(conn).unwrap();
    guest.inventory[guest.selected]
        .as_ref()
        .map(|s| s.item.to_string())
}

/// Two guests click one cod with a water bucket in the same tick. The first scoops it up
/// (`Bucketable.bucketMobPickup` ends in `discard`); the second click names an entity
/// `ServerGamePacketListenerImpl.handleInteract` no longer finds, so its bucket stays water.
#[test]
#[ignore = "daily"]
fn a_fish_scooped_up_this_tick_is_not_scooped_up_again() {
    crate::game::native_guest_turn_tests::with_guests(|game| {
        let cod = game.res.mob_types().get("minecraft:cod").unwrap().clone();
        game.mobs
            .push(Mob::new(40, cod, Vec3::new(3.0, 64.0, 6.0), 0.0));
        for conn in [12, 13] {
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(conn, |g| {
                    g.inventory[0] = Some(ItemStack::new("minecraft:water_bucket", 1));
                    g.selected = 0;
                })
                .unwrap();
        }
        let interact = || {
            Serverbound::Interact(Interact {
                id: 40,
                hand: 0,
                location: [0.0; 3],
                secondary: false,
            })
        };
        game.lan_guest_packet(12, interact());
        game.lan_guest_packet(13, interact());
        assert_eq!(held(game, 12).as_deref(), Some("minecraft:cod_bucket"));
        assert_eq!(held(game, 13).as_deref(), Some("minecraft:water_bucket"));
        assert_eq!(
            game.guest_attack(13, 1, 40),
            Err(ActionRefused::InvalidTarget)
        );
    });
}
