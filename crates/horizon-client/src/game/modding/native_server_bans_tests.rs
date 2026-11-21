//! `server_ban_list` without a dedicated server: no entries, for either list.
use super::native_guest_turn_tests::with_guests;
use mod_api::BanListKind;

#[test]
#[ignore = "daily"]
fn a_mod_reads_no_ban_entry_without_a_dedicated_server() {
    with_guests(|game| {
        // A guest game keeps no server lists, so both ban lists are empty.
        assert!(game.server_ban_list(BanListKind::Players).is_empty());
        assert!(game.server_ban_list(BanListKind::Ips).is_empty());
    });
}
