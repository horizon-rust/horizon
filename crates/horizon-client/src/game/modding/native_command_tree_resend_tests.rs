//! `player_resend_command_tree` against real LAN guests: one player's command tree goes to that
//! player's client alone, and each refusal is its own name with nothing sent.

use super::native_entity_refs::tests::with_client_game;
use super::native_guest_turn_tests::with_guests;
use horizon_client::lan::{Out, PlayerSender};
use horizon_protocol::{buf::Reader, ids};
use mod_api::{EntityRef, PlayerRef, PlayerWorldRefusal as Refusal, WorldEpoch};
use std::sync::mpsc;

/// The handle of a player of an earlier open of the world.
fn stale(player: PlayerRef) -> PlayerRef {
    let entity = player.entity();
    mod_api::__private::game::player_ref(
        EntityRef::new(WorldEpoch::new(entity.epoch().get() + 1), entity.id()),
        player.uuid(),
    )
}

/// A fake player: no connection, not in the server's list.
fn fake(game: &mut super::Game<'_>) -> PlayerRef {
    game.next_entity = 100;
    let level = game.level(game.dim.id()).unwrap();
    game.fake_player(level, 1234, "machine").unwrap()
}

/// Gives the guest on `conn` a channel of its own and returns what its client is sent.
fn listen(game: &mut super::Game<'_>, conn: u32) -> mpsc::Receiver<Out> {
    let (tx, rx) = mpsc::channel();
    game.lan
        .as_mut()
        .unwrap()
        .hub
        .guests
        .update_connection(conn, |g| {
            g.sender = PlayerSender::new(tx, Default::default());
        });
    rx
}

/// How many command trees (`ClientboundCommandsPacket`) the packets in `rx` hold.
fn trees(rx: &mpsc::Receiver<Out>) -> usize {
    let mut count = 0;
    for out in rx.try_iter() {
        let bodies = match out {
            Out::Packet(body) => vec![body],
            Out::Packets(bodies) => bodies,
            Out::Disconnect(_) => panic!("unexpected disconnect"),
        };
        for body in bodies {
            let mut r = Reader::new(&body);
            if r.varint().unwrap() == ids::play::clientbound::COMMANDS {
                count += 1;
            }
        }
    }
    count
}

/// The named player gets one tree again; the other online player gets none.
#[test]
#[ignore = "daily"]
fn resend_reaches_only_the_named_player() {
    with_guests(|game| {
        let b = game.player(13).unwrap();
        let rx_a = listen(game, 12);
        let rx_b = listen(game, 13);
        assert_eq!(game.player_resend_command_tree(b), Ok(()));
        assert_eq!(trees(&rx_b), 1, "the named player's tree is sent once");
        assert_eq!(trees(&rx_a), 0, "the other player is sent nothing");
    });
}

/// An unknown or stale player is `Stale`, a fake one `NoConnection`; none of them is sent a tree.
#[test]
#[ignore = "daily"]
fn each_resend_refusal_has_its_own_name_and_sends_nothing() {
    with_guests(|game| {
        let b = game.player(13).unwrap();
        let rx = listen(game, 13);
        let unknown = mod_api::__private::game::player_ref(b.entity(), 77);
        assert_eq!(
            game.player_resend_command_tree(unknown),
            Err(Refusal::Stale)
        );
        assert_eq!(
            game.player_resend_command_tree(stale(b)),
            Err(Refusal::Stale)
        );
        let machine = fake(game);
        assert_eq!(
            game.player_resend_command_tree(machine),
            Err(Refusal::NoConnection)
        );
        assert_eq!(trees(&rx), 0, "a refused resend sends nothing");
    });
}

/// On a client the resend answers `Client`.
#[test]
#[ignore = "daily"]
fn a_client_cannot_resend_a_command_tree() {
    with_client_game(|game| {
        let ghost = mod_api::__private::game::player_ref(EntityRef::new(WorldEpoch::new(0), 1), 1);
        assert_eq!(game.player_resend_command_tree(ghost), Err(Refusal::Client));
    });
}

/// A resend asked from inside `player_commands_sent` (the server is filtering the trees) starts
/// no second pass and sends nothing itself: the pass that asked the hook sends the tree.
#[test]
#[ignore = "daily"]
fn a_resend_from_inside_the_filter_pass_does_nothing() {
    with_guests(|game| {
        let b = game.player(13).unwrap();
        let rx = listen(game, 13);
        game.native_commands.filtering = true;
        assert_eq!(game.player_resend_command_tree(b), Ok(()));
        game.native_filter_commands(Some(13));
        assert_eq!(trees(&rx), 0, "a nested resend sends no tree");
        assert!(
            game.native_commands.filtering,
            "a nested pass leaves the running pass's flag alone"
        );
        game.native_commands.filtering = false;
        // The guard is released after a pass, so a later resend works again.
        assert_eq!(game.player_resend_command_tree(b), Ok(()));
        assert_eq!(trees(&rx), 1);
        assert!(!game.native_commands.filtering);
    });
}
