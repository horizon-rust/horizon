//! `player_advancement_done_cancel` heard through the game's own completion: a listener's cancel is
//! reported back to the completion, which then withholds its toast, announcement and rewards.
use mod_api::{Flow, Hooks, PlayerAdvancementDoneCancelEvent, ServerHost};

struct CancelCompletion;
#[mod_api::__subscriptions]
impl Hooks for CancelCompletion {
    fn player_advancement_done_cancel(
        &self,
        _host: &mut dyn ServerHost,
        event: &mut PlayerAdvancementDoneCancelEvent<'_>,
    ) -> Flow {
        assert_eq!(event.player, 7);
        assert_eq!(event.name, "catcher");
        assert_eq!(event.advancement, "minecraft:story/mine_stone");
        Flow::Cancel
    }
}

#[test]
#[ignore = "daily"]
fn a_canceled_advancement_completion_is_withheld() {
    mod_platform::testing::with_mods(
        vec![mod_api::ModEntry::new("cancel_award", Box::new(CancelCompletion))].into(),
        || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let mut cancel = false;
                game.native_player_advancement_done_cancel(
                    7,
                    "catcher",
                    "minecraft:story/mine_stone",
                    &mut cancel,
                );
                assert!(cancel);
            });
        },
    );
}
