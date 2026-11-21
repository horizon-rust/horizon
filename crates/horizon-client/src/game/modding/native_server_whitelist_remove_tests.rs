//! `whitelist_remove` without a dedicated server: nothing is removed.
use super::native_guest_turn_tests::with_guests;

#[test]
#[ignore = "daily"]
fn a_mod_removes_no_whitelist_entry_without_a_dedicated_server() {
    with_guests(|game| {
        assert!(!game.whitelist_remove(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef));
        assert!(!game.whitelist_remove(0));
    });
}
