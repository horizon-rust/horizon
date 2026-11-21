//! `whitelist_add` without a dedicated server, and for names no player can have: nothing is added.
use super::native_guest_turn_tests::with_guests;

#[test]
#[ignore = "daily"]
fn a_mod_adds_no_whitelist_entry_without_a_dedicated_server() {
    with_guests(|game| {
        assert!(!game.whitelist_add(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef, "Steve"));
        assert!(!game.whitelist_add(0, "Steve"));
    });
}

#[test]
#[ignore = "daily"]
fn a_name_no_player_can_have_is_refused() {
    with_guests(|game| {
        assert!(!game.whitelist_add(1, ""));
        // 17 characters: one over the 16 a player name allows.
        assert!(!game.whitelist_add(1, "seventeen_chars_x"));
        assert!(!game.whitelist_add(1, "bad name"));
    });
}
