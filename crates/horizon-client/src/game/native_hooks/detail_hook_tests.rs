#![cfg(test)]
//! The separate hooks of the follow-up PR, each heard through its native function with literal
//! values: the listener sees the values, and its cancel comes back.
use super::*;
use crate::game::native_entity_refs::tests::with_game;
use mod_api::{
    BreakBlockPlayerEvent, EntityJoinLevelReasonEvent, Flow, Hooks, LeftClickBlockPlayerEvent,
    ModEntry, PlayerOpenSignCancelEvent, PlayerTakeLecternBookCancelEvent,
    PrepareHopperTransferCancelEvent, ServerHost,
};

struct Probe;
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn break_block_player(&self, _: &mut dyn ServerHost, event: &mut BreakBlockPlayerEvent) -> Flow {
        assert_eq!(event.pos, [1, 64, 2]);
        assert_eq!(event.player, Some(7));
        Flow::Cancel
    }
    fn left_click_block_player(
        &self,
        _: &mut dyn ServerHost,
        event: &mut LeftClickBlockPlayerEvent,
    ) -> Flow {
        assert_eq!(event.pos, [3, 65, 4]);
        assert_eq!(event.face, [0, 1, 0]);
        assert_eq!(event.player, 9);
        Flow::Cancel
    }
    fn entity_join_level_reason(
        &self,
        _: &mut dyn ServerHost,
        event: &mut EntityJoinLevelReasonEvent,
    ) -> Flow {
        assert_eq!(event.dimension, "minecraft:overworld");
        assert_eq!(event.entity, 77);
        assert_eq!(event.reason, Some(mod_api::SpawnReason::Spawner));
        Flow::Cancel
    }
    fn player_open_sign_cancel(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerOpenSignCancelEvent,
    ) -> Flow {
        assert_eq!((event.player, event.name, event.pos, event.front), (5, "scribe", [8, 70, 9], true));
        Flow::Cancel
    }
    fn player_take_lectern_book_cancel(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerTakeLecternBookCancelEvent,
    ) -> Flow {
        assert_eq!((event.player, event.pos, event.book), (5, [2, 64, 2], "minecraft:writable_book"));
        Flow::Cancel
    }
    fn prepare_hopper_transfer_cancel(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PrepareHopperTransferCancelEvent,
    ) -> Flow {
        assert_eq!(event.pos, [6, 63, 6]);
        Flow::Cancel
    }
}

#[test]
#[ignore = "daily"]
fn the_cancel_of_each_separate_hook_comes_back() {
    let mods: mod_api::Mods = vec![ModEntry::new("probe", Box::new(Probe))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let state = horizon_world::registry::BlockStateId(0);
            let (mut canceled, mut exp) = (false, None);
            game.native_break_block_player([1, 64, 2], state, Some(7), &mut canceled, &mut exp);
            assert!(canceled);
            let mut canceled = false;
            game.native_left_click_block_player([3, 65, 4], [0, 1, 0], state, 9, &mut canceled);
            assert!(canceled);
            let mut canceled = false;
            game.native_entity_join_level_reason(mod_api::__private::EntityJoinLevelReasonCall {
                dimension: "minecraft:overworld",
                entity: 77,
                kind: "minecraft:zombie",
                pos: [0.5, 64.0, 0.5],
                loaded_from_disk: false,
                reason: Some(mod_api::SpawnReason::Spawner),
                canceled: &mut canceled,
            });
            assert!(canceled);
            let mut cancel = false;
            game.native_player_open_sign_cancel(5, "scribe", [8, 70, 9], true, &mut cancel);
            assert!(cancel);
            let mut cancel = false;
            game.native_player_take_lectern_book_cancel(
                5,
                "reader",
                [2, 64, 2],
                "minecraft:writable_book",
                &mut cancel,
            );
            assert!(cancel);
            let mut cancel = false;
            game.native_prepare_hopper_transfer_cancel([6, 63, 6], &mut cancel);
            assert!(cancel);
        });
    });
}
