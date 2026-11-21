use super::*;
use crate::game::native_entity_refs::tests::with_game;
use horizon_client::chunk::Chunk;
use mod_api::{Hooks, ModEntry, PortalCreateDetailEvent, ServerHost};
use std::sync::{Arc, Mutex};

/// Hears `portal_create_detail`: the dimension, the origin and how many blocks were listed.
struct Probe(Arc<Mutex<Vec<(String, [i32; 3], usize)>>>);
#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn portal_create_detail(&self, _: &mut dyn ServerHost, event: &mut PortalCreateDetailEvent<'_>) {
        self.0
            .lock()
            .unwrap()
            .push((event.dimension.to_owned(), event.pos, event.created.len()));
    }
}

/// An End arrival builds the platform and tells `portal_create_detail` with an empty block list.
#[test]
#[ignore = "daily"]
fn an_end_platform_is_told_with_an_empty_block_list() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![ModEntry::new("portal", Box::new(Probe(Arc::clone(&heard))))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.enter_level(Dim::End).unwrap();
            for cz in -1..=0 {
                game.world
                    .chunks
                    .insert((END_SPAWN[0] >> 4, cz), Arc::new(Chunk::empty(END_SPAWN[0] >> 4, cz)));
            }
            game.end_arrival(Vec3::ZERO, &mut None).unwrap();
        });
    });
    let origin = [END_SPAWN[0], END_SPAWN[1] - 1, END_SPAWN[2]];
    assert_eq!(
        *heard.lock().unwrap(),
        [("minecraft:the_end".to_owned(), origin, 0)]
    );
}
