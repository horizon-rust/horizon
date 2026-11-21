//! `PlayerRef::permission_bool` against a real server world: a handle kept from an earlier open of
//! the world names the same UUID but is no longer this server's player.

use super::native_guest_turn_tests::with_guests;
use mod_api::{
    EntityRef, NodeDefault, PermissionError, PermissionLevel, PermissionNode, PermissionValueType,
    PlayerRef, WorldEpoch,
};

/// The handle of a player of an earlier open of the world: the same UUID, an older epoch.
fn stale(player: PlayerRef) -> PlayerRef {
    let entity = player.entity();
    mod_api::__private::game::player_ref(
        EntityRef::new(WorldEpoch::new(entity.epoch().get() + 1), entity.id()),
        player.uuid(),
    )
}

#[test]
#[ignore = "daily"]
fn a_stale_handle_with_the_same_uuid_is_an_unknown_player_and_the_current_one_is_answered() {
    with_guests(|game| {
        let node = PermissionNode::new("a.flag", PermissionValueType::Boolean);
        let levels = game.permission_query_state.level_source(|_| None);
        game.permission_service.set_level_source(levels);
        game.permission_service
            .register_node(node.clone(), NodeDefault::Level(PermissionLevel::All))
            .unwrap();
        game.permission_service.finish_gathering(None);
        let player = game.player(13).unwrap();
        assert_eq!(player.permission_bool(game, &node), Ok(true));
        assert_eq!(
            stale(player).permission_bool(game, &node),
            Err(PermissionError::UnknownPlayer)
        );
    });
}
