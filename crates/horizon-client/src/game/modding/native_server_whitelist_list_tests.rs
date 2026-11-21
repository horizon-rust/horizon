//! `server_whitelist` without a dedicated server: the whitelist has no entries.
use super::native_guest_turn_tests::with_guests;

#[test]
#[ignore = "daily"]
fn a_mod_reads_no_whitelist_entries_without_a_dedicated_server() {
    with_guests(|game| {
        // A guest game keeps no server lists, so the whitelist is empty.
        assert!(game.server_whitelist().is_empty());
    });
}
