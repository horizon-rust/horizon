//! `entity_set_saved`: an entity a mod kept out of the saves is not written, by the autosave or by
//! its chunk's unload, until it is put back.
use std::{collections::HashSet, sync::Arc};

use horizon_client::chunk::Chunk;
use horizon_core::nbt::Compound;

use super::Game;

#[test]
#[ignore = "daily"]
fn an_entity_kept_out_of_the_saves_is_not_written_until_put_back() {
    super::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_joins_skip();
        let dimension = game.dim.id().to_owned();
        let keys = HashSet::from([(0, 0)]);
        let summon = |game: &mut Game<'_>, kind: &str| {
            let id = game
                .summon(&dimension, kind, [1.5, 70.0, 1.5], Compound::new())
                .expect("a summoned entity");
            game.entity(id).expect("its handle")
        };
        let kept = summon(game, "minecraft:text_display");
        summon(game, "minecraft:item_display");
        let pig = summon(game, "minecraft:pig");
        let saved = |game: &mut Game<'_>| -> Vec<String> {
            let mut kinds: Vec<String> = game
                .entity_chunks(&keys, false)
                .iter()
                .flat_map(|(_, tags)| tags)
                .filter_map(horizon_client::entity_nbt::kind)
                .map(|kind| kind.to_string())
                .collect();
            kinds.sort();
            kinds
        };
        assert_eq!(saved(game).len(), 3);
        assert!(game.entity_set_saved(kept, false));
        assert!(game.entity_set_saved(pig, false));
        assert_eq!(saved(game), ["minecraft:item_display"]);
        assert!(game.entity_set_saved(pig, true));
        assert_eq!(saved(game), ["minecraft:item_display", "minecraft:pig"]);
        // A handle that is gone is refused.
        game.mobs.retain(|mob| mob.id != pig.id());
        assert!(!game.entity_set_saved(pig, false));
        // Unloading the chunk writes what is left, and takes the kept entity away unwritten.
        let unloaded = game.entity_chunks(&keys, true);
        assert_eq!(unloaded.iter().map(|(_, tags)| tags.len()).sum::<usize>(), 1);
    });
}
