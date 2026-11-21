//! `serverbound_packet`: a listened packet is held for the game's ruling, in order, and the ruling
//! decides whether the hub handles it.
use super::pending_move_tests::*;
use super::*;

/// A guest at `(0, 64, 0)` with `inbox` waiting and a listener on its packets.
fn listening(inbox: Vec<S>) -> Hub {
    let mut hub = standing(false, inbox);
    hub.packets_listened = true;
    hub
}

fn heard(events: &[HostEvent]) -> Vec<u32> {
    events
        .iter()
        .filter_map(|e| match e {
            HostEvent::PacketHeard(conn) => Some(*conn),
            _ => None,
        })
        .collect()
}

#[test]
fn a_packet_waits_for_the_ruling_and_the_next_is_asked_only_after_it() {
    let mut hub = listening(vec![walking(1.0), tick_end()]);
    let conn = hub.guests[0].conn;
    assert_eq!(heard(&drain(&mut hub)), [conn], "the first is held");
    assert!(hub.packets_paused());
    assert_eq!(
        hub.guests[0].position,
        [0.0, 64.0, 0.0],
        "nothing applied yet"
    );
    assert_eq!(hub.guests[0].inbox.len(), 1, "the second keeps its place");
    let (_, packet) = hub.heard_packet(conn).unwrap();
    assert!(matches!(packet, S::MovePlayerPos(p) if p.x == 1.0));
    hub.rule_packet(conn, true);
    assert!(hub.heard_packet(conn).is_none());
    // The passed packet is handled unasked, then the next one is held.
    assert_eq!(heard(&drain(&mut hub)), [conn]);
    assert_eq!(hub.guests[0].position, [1.0, 64.0, 0.0]);
    let (_, packet) = hub.heard_packet(conn).unwrap();
    assert!(matches!(packet, S::ClientTickEnd(_)));
}

#[test]
fn a_cancelled_packet_is_dropped_and_the_ones_behind_it_go_on() {
    let mut hub = listening(vec![walking(1.0), turned(30.0)]);
    let conn = hub.guests[0].conn;
    assert_eq!(heard(&drain(&mut hub)), [conn]);
    hub.rule_packet(conn, false);
    assert_eq!(hub.guests[0].inbox.len(), 1);
    assert_eq!(heard(&drain(&mut hub)), [conn], "the turn is asked next");
    assert_eq!(
        hub.guests[0].position,
        [0.0, 64.0, 0.0],
        "the move never happened"
    );
    hub.rule_packet(conn, true);
    drain(&mut hub);
    assert_eq!(hub.guests[0].yaw, 30.0);
    assert_eq!(hub.guests[0].position, [0.0, 64.0, 0.0]);
}

#[test]
fn with_no_listener_nothing_is_held() {
    let mut hub = standing(false, vec![walking(1.0), tick_end()]);
    assert!(heard(&drain(&mut hub)).is_empty());
    assert!(!hub.packets_paused());
    assert_eq!(hub.guests[0].position, [1.0, 64.0, 0.0]);
}

#[test]
fn a_ruling_for_nobody_changes_nothing() {
    let mut hub = listening(vec![walking(1.0)]);
    let conn = hub.guests[0].conn;
    hub.rule_packet(conn, true);
    hub.rule_packet(conn + 100, false);
    assert!(hub.heard_packet(conn).is_none());
    assert_eq!(hub.guests[0].inbox.len(), 1);
    // A ruling for another connection does not release the held packet.
    assert_eq!(heard(&drain(&mut hub)), [conn]);
    hub.rule_packet(conn + 100, true);
    assert!(hub.heard_packet(conn).is_some());
}

#[test]
fn a_packet_held_for_a_guest_who_left_is_dropped_with_its_ruling() {
    let mut hub = listening(vec![walking(1.0)]);
    let conn = hub.guests[0].conn;
    assert_eq!(heard(&drain(&mut hub)), [conn]);
    hub.guests.remove(0);
    assert!(hub.heard_packet(conn).is_none(), "no player to name");
    hub.rule_packet(conn, true);
    assert!(hub.guests.is_empty());
    assert!(hub.heard.is_none(), "the slot is free again");
}
