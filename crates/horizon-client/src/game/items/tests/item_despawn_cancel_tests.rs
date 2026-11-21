//! `item_despawn_detail` cancel: a dropped item at its 6000-tick lifetime that a listener cancels
//! stays for another 6000 ticks (Paper's cancel of `ItemDespawnDetailEvent`); without a cancel it is removed.
use crate::game::Dim;
use crate::game::native_entity_refs::tests::with_game;
use horizon_client::{chunk::Chunk, items::ItemStack};
use mod_api::{Flow, Hooks, ItemDespawnDetailEvent, ModEntry, ServerHost};
use std::sync::{Arc, Mutex};

/// Hears `item_despawn_detail` and cancels it while `cancel` is set.
struct CancelDespawn {
    cancel: bool,
    heard: Arc<Mutex<Vec<u32>>>,
}

#[mod_api::__subscriptions]
impl Hooks for CancelDespawn {
    fn item_despawn_detail(&self, _: &mut dyn ServerHost, event: &mut ItemDespawnDetailEvent<'_>) -> Flow {
        self.heard.lock().unwrap().push(event.entity);
        if self.cancel {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

/// Runs one item tick on a dedicated server with one dropped apple at `age`, and returns its
/// age afterwards (None when it was removed) and the entity ids the listener heard.
fn tick_at_lifetime(cancel: bool) -> (Option<i32>, Vec<u32>) {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "item-despawn-cancel",
        Box::new(CancelDespawn {
            cancel,
            heard: heard.clone(),
        }),
    )]
    .into();
    let mut out = (None, Vec::new());
    mod_platform::testing::with_mods(mods, || {
        let root = std::env::temp_dir().join(format!(
            "horizon-item-despawn-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        with_game(|game| {
            game.world_root = root.clone();
            game.enter_level(Dim::Overworld).unwrap();
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.push_item_entity(
                ItemStack::new("minecraft:apple", 1),
                [1.5, 64.0, 1.5],
                Some([0.0, 0.0, 0.0]),
                0,
                None,
            );
            let id = game.dropped.last().unwrap().id;
            game.ticked_entities_through = id;
            game.dropped.last_mut().unwrap().age = 6000;
            let mut round = game.native_tick_round();
            game.tick_dropped_items(&mut round);
            let age = game.dropped.iter().find(|item| item.id == id).map(|item| item.age);
            out = (age, heard.lock().unwrap().clone());
        });
        let _ = std::fs::remove_dir_all(&root);
    });
    out
}

#[test]
#[ignore = "daily"]
fn a_cancelled_item_despawn_keeps_the_item_for_another_lifetime() {
    let (age, heard) = tick_at_lifetime(true);
    assert_eq!(heard.len(), 1, "the listener heard the despawn once");
    assert_eq!(age, Some(0), "the cancel grants 6000 more ticks");
}

#[test]
#[ignore = "daily"]
fn an_uncancelled_item_despawn_removes_the_item() {
    let (age, heard) = tick_at_lifetime(false);
    assert_eq!(heard.len(), 1, "the listener heard the despawn once");
    assert_eq!(age, None, "age 6000 without a cancel discards it");
}
