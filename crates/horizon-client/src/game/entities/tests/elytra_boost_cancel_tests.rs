use mod_api::{Flow, Hooks, ModEntry, PlayerElytraBoostCancelEvent, ServerHost};

struct CancelBoost;
#[mod_api::__subscriptions]
impl Hooks for CancelBoost {
    fn player_elytra_boost_cancel(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerElytraBoostCancelEvent<'_>,
    ) -> Flow {
        assert_eq!(event.player, 7);
        assert_eq!(event.name, "glider");
        Flow::Cancel
    }
}

/// A listener of `player_elytra_boost_cancel` hears the player and its cancel comes back.
#[test]
#[ignore = "daily"]
fn a_canceled_elytra_boost_is_reported_back() {
    mod_platform::testing::with_mods(
        vec![ModEntry::new("cancel_boost", Box::new(CancelBoost))].into(),
        || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let mut cancel = false;
                game.native_player_elytra_boost_cancel(7, "glider", &mut cancel);
                assert!(cancel);
            });
        },
    );
}
