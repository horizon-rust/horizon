use crate::game::native_entity_refs::tests::with_game;
use crate::game::native_item_places::tests::{guest, open_to_lan};
use mod_api::{EntityToggleGlideCancelEvent, Flow, Hooks, ModEntry, ServerHost};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Hears every glide change before it is applied and cancels it while `cancel` is set.
struct GlideProbe {
    cancel: Arc<AtomicBool>,
    heard: Arc<Mutex<Vec<(u128, String, bool)>>>,
}
#[mod_api::__subscriptions]
impl Hooks for GlideProbe {
    fn entity_toggle_glide_cancel(
        &self,
        _: &mut dyn ServerHost,
        event: &mut EntityToggleGlideCancelEvent<'_>,
    ) -> Flow {
        self.heard
            .lock()
            .unwrap()
            .push((event.player, event.name.to_owned(), event.gliding));
        if self.cancel.load(Ordering::SeqCst) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
}

/// `EntityToggleGlideCancelEvent`: a gliding guest's second start ends the glide. The hook is heard
/// before the change with the new state (`false`); a cancel keeps the guest gliding, and without
/// one the glide ends.
#[test]
#[ignore = "daily"]
fn a_mod_cancels_a_guests_glide_toggle() {
    let cancel = Arc::new(AtomicBool::new(true));
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "glide-probe",
        Box::new(GlideProbe {
            cancel: Arc::clone(&cancel),
            heard: Arc::clone(&heard),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            open_to_lan(game);
            let mut gliding = guest(5);
            gliding.fall_flying = true;
            game.lan.as_mut().unwrap().hub.guests.push(gliding);
            game.lan_start_gliding(5);
            assert_eq!(
                *heard.lock().unwrap(),
                [(127, "guest5".to_owned(), false)]
            );
            assert!(game.lan.as_ref().unwrap().hub.guest(5).unwrap().fall_flying);
            cancel.store(false, Ordering::SeqCst);
            game.lan_start_gliding(5);
            assert_eq!(heard.lock().unwrap().len(), 2);
            assert!(!game.lan.as_ref().unwrap().hub.guest(5).unwrap().fall_flying);
        });
    });
}
