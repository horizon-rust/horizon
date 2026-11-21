//! `server_is_whitelisted` without a dedicated server: nobody is on a list that is not loaded.
use super::native_guest_turn_tests::with_guests;

#[test]
#[ignore = "daily"]
fn a_mod_reads_no_whitelist_entry_without_a_dedicated_server() {
    with_guests(|game| {
        // A guest game keeps no server list, so every UUID answers false, the one reason there is.
        assert!(!game.server_is_whitelisted(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef));
        assert!(!game.server_is_whitelisted(0));
    });
}
