use super::*;
use crate::game::native_entity_refs::tests::with_game;
use mod_api::{EntityBreakDoorCancelEvent, Flow, Hooks, ModEntry, ServerHost};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Hears every door a mob is about to break and cancels it while `cancel` is set.
struct DoorProbe {
    cancel: Arc<AtomicBool>,
    heard: Arc<Mutex<Vec<(u32, String, [i32; 3])>>>,
}
#[mod_api::__subscriptions]
impl Hooks for DoorProbe {
    fn entity_break_door_cancel(
        &self,
        _: &mut dyn ServerHost,
        event: &mut EntityBreakDoorCancelEvent<'_>,
    ) -> Flow {
        self.heard
            .lock()
            .unwrap()
            .push((event.entity, event.kind.to_owned(), event.pos));
        if self.cancel.load(Ordering::SeqCst) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

/// `EntityBreakDoorCancelEvent`: a zombie's `BreakDoorGoal` event is heard before the door goes, with the
/// zombie, its type and the door's cell. A cancel keeps the door; without one the door becomes air.
#[test]
#[ignore = "daily"]
fn a_mod_cancels_a_mob_breaking_a_door() {
    let cancel = Arc::new(AtomicBool::new(true));
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "door-probe",
        Box::new(DoorProbe {
            cancel: Arc::clone(&cancel),
            heard: Arc::clone(&heard),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            use horizon_client::chunk::Chunk;
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.build_registries();
            let door = game
                .pack
                .assets
                .registry
                .default_state("minecraft:oak_door")
                .unwrap();
            let pos = [6, 65, 3];
            game.world.set(pos[0], pos[1], pos[2], door).unwrap();
            let id = game
                .spawn_mob_raw("minecraft:zombie", [3.5, 65.0, 3.5], true)
                .unwrap();
            game.handle_mob_events(vec![MobEvent::BreakDoor { mob: id, pos }])
                .unwrap();
            assert_eq!(
                *heard.lock().unwrap(),
                [(id, "minecraft:zombie".to_owned(), pos)]
            );
            assert_eq!(game.world.block(pos[0], pos[1], pos[2]), door);
            cancel.store(false, Ordering::SeqCst);
            game.handle_mob_events(vec![MobEvent::BreakDoor { mob: id, pos }])
                .unwrap();
            assert_eq!(heard.lock().unwrap().len(), 2);
            assert_eq!(
                game.world.block(pos[0], pos[1], pos[2]),
                game.pack.assets.registry.default_state("minecraft:air").unwrap()
            );
        });
    });
}
