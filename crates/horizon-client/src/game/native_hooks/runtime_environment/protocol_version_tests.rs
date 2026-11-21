//! The protocol number and its name as a mod reads them: literal values for 26.3 (protocol 777).
use super::protocol_name_of;

#[test]
fn the_game_protocol_is_777_for_26_3() {
    assert_eq!(horizon_core::version::PROTOCOL_VERSION, 777);
}

#[test]
fn only_the_games_own_protocol_has_a_name() {
    assert_eq!(protocol_name_of(777), Some("26.3"));
    assert_eq!(protocol_name_of(776), None);
    assert_eq!(protocol_name_of(0), None);
    assert_eq!(protocol_name_of(-1), None);
}
