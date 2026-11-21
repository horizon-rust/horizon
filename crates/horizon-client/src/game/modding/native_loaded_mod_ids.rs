//! `loaded_mod_ids`: the ids of the mods loaded at startup, in dispatch order.
use super::Game;

impl Game<'_> {
    /// Every loaded package's id (`mod_api::__private::loaded_mods`), fixed at startup.
    pub(super) fn loaded_mod_ids(&self) -> Vec<String> {
        mod_api::__private::loaded_mods()
            .ids()
            .map(str::to_owned)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;

    /// The list is the startup inventory: it names no id that is not loaded, and an absent id
    /// is not in it.
    #[test]
    #[ignore = "daily"]
    fn loaded_mod_ids_lists_the_startup_inventory() {
        with_guests(|game| {
            let ids = game.loaded_mod_ids();
            let inventory: Vec<String> = mod_api::__private::loaded_mods()
                .ids()
                .map(str::to_owned)
                .collect();
            assert_eq!(ids, inventory);
            assert!(!ids.iter().any(|id| id == "absent_mod_fixture"));
        });
    }
}
