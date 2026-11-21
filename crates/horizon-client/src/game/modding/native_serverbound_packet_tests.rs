//! `serverbound_packet`: the hook hears each packet a player's client sent with the exact type id,
//! in the order sent, and a cancel drops that packet only. The cases run the real game tick.
use super::native_guest_turn_tests::with_guests;
use horizon_client::lan::guest::InboxItem;
use horizon_protocol::packets::play::{self, Serverbound as S};
use mod_api::{Flow, Hooks, ModEntry, ServerHost, ServerboundPacketEvent};
use std::sync::{Arc, Mutex};

type Heard = Arc<Mutex<Vec<(u128, String)>>>;

/// Records each packet and cancels the turns of the head (`move_player_rot`).
struct Filter(Heard);

#[mod_api::__subscriptions]
impl Hooks for Filter {
    fn serverbound_packet(
        &self,
        _: &mut dyn ServerHost,
        event: &mut ServerboundPacketEvent<'_>,
    ) -> Flow {
        self.0
            .lock()
            .unwrap()
            .push((event.player, event.packet.id.to_string()));
        if event.packet.id.to_string() == "minecraft:move_player_rot" {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

/// Guest 13 is loaded, long past its first tick, with a turn of its head (yaw 40) and the end of
/// its client tick in its inbox.
fn turn_and_tick_end_waiting(game: &mut super::Game<'_>) {
    let hub = &mut game.lan.as_mut().unwrap().hub;
    hub.ticks = 10;
    hub.update_guest(13, |g| {
        g.load_timeout = 0;
        g.first_good = g.position;
        (g.yaw, g.pitch) = (0.0, 0.0);
        g.level_entered = 0;
        g.last_reported = (g.position, [0.0, 0.0]);
        let turn = S::MovePlayerRot(play::MovePlayerRot {
            yaw: 40.0,
            pitch: 0.0,
            flags: 1,
        });
        g.inbox.push_back(InboxItem::Packet(turn, None));
        let end = S::ClientTickEnd(horizon_protocol::packets::Empty);
        g.inbox.push_back(InboxItem::Packet(end, None));
    });
}

fn game_tick(game: &mut super::Game<'_>) {
    game.level_pass.packets_drained = false;
    game.tick_lan_packets();
}

fn yaw_and_inbox(game: &super::Game<'_>) -> (f32, usize) {
    let g = game.lan.as_ref().unwrap().hub.guest(13).unwrap();
    (g.yaw, g.inbox.len())
}

#[test]
#[ignore = "daily"]
fn each_packet_is_heard_in_order_and_a_cancel_drops_only_that_packet() {
    let heard = Heard::default();
    let mods = vec![ModEntry::new("filter", Box::new(Filter(heard.clone())))].into();
    mod_platform::testing::with_mods(mods, || {
        with_guests(|game| {
            turn_and_tick_end_waiting(game);
            game_tick(game);
            assert_eq!(
                std::mem::take(&mut *heard.lock().unwrap()),
                [
                    (13, "minecraft:move_player_rot".to_string()),
                    (13, "minecraft:client_tick_end".to_string())
                ]
            );
            assert_eq!(yaw_and_inbox(game), (0.0, 0));
        });
    });
}

#[test]
#[ignore = "daily"]
fn with_no_listener_the_packet_is_handled_in_place() {
    with_guests(|game| {
        turn_and_tick_end_waiting(game);
        game_tick(game);
        assert_eq!(yaw_and_inbox(game), (40.0, 0));
        assert!(game.lan.as_ref().unwrap().hub.heard_packet(13).is_none());
    });
}
