//! `ban_changed`: nothing is queued while no mod listens, and a ban's record keeps its fields.
use super::native_server_bans::player_record;
use horizon_server_files::user_lists::{BanEntry, NameAndId};
use mod_api::BanListKind;

#[test]
#[ignore = "daily"]
fn a_change_is_kept_only_while_a_mod_listens() {
    let ban = BanEntry::new(
        NameAndId {
            id: 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef,
            name: "Steve".into(),
        },
        Some("Server".into()),
        None,
        Some("griefing".into()),
    );
    // No mod in this test listens, so the change is not kept.
    super::native_ban_changes::queue(BanListKind::Players, true, player_record(&ban));
    assert!(super::native_ban_changes::PENDING.lock().unwrap().is_empty());
}

#[test]
#[ignore = "daily"]
fn a_name_ban_keeps_its_fields_in_the_record() {
    let id = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
    let ban = BanEntry::new(
        NameAndId {
            id,
            name: "Steve".into(),
        },
        Some("Server".into()),
        Some(1_900_000_000_000),
        Some("griefing".into()),
    );
    let record = player_record(&ban);
    assert_eq!(record.uuid, Some(id));
    assert_eq!(record.name.as_deref(), Some("Steve"));
    assert_eq!(record.ip, None);
    assert_eq!(record.source, "Server");
    assert_eq!(record.expires_millis, Some(1_900_000_000_000));
    assert_eq!(record.reason.as_deref(), Some("griefing"));
}
