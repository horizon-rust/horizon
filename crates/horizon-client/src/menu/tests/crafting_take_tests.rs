//! `ResultSlot.remove` counts what was asked for, not the whole stack the
//! result container hands over.
use super::{stack, with_menu};
use crate::menu::{Input, Kind};

/// A right click asks for half the result, and `ResultContainer.removeItem`
/// still hands over all of it: vanilla awards `min(amount, count)` = 2 for
/// four planks, not 4.
#[test]
fn a_secondary_click_counts_only_the_amount_it_asked_for() {
    with_menu(Kind::Crafting, |env, menu| {
        menu.creative = false;
        menu.set_slot(env, 1, stack("oak_log", 1));
        assert_eq!(menu.slot(0).map(|s| s.count), Some(4));
        menu.click(env, 0, 1, Input::Pickup).unwrap();
        // The whole result stack goes to the cursor.
        assert_eq!(menu.carried.as_ref().map(|s| s.count), Some(4));
        let counted: Vec<_> = menu
            .crafted_counts
            .iter()
            .map(|(item, count)| (item.to_string(), *count))
            .collect();
        assert_eq!(counted, [("minecraft:oak_planks".to_owned(), 2)]);
    });
}
