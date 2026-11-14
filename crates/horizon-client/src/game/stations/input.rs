//! A station's slot changes: its result and, for the anvil, the rename box
//! follow the input slots.
use super::super::Game;

use horizon_protocol::packets::play::{self, Serverbound as S};

impl Game<'_> {
    /// Recompute after slot changes; the anvil box follows its input item and a
    /// changed name goes to the server, as `AnvilScreen.slotChanged` does
    /// through `EditBox.setValue` and its responder.
    pub(super) fn station_after_input(&mut self) {
        let Some(s) = self.station.as_mut() else {
            return;
        };
        let sent = s.after_input(&self.pack.names);
        self.station_changed();
        // `AnvilScreen.onNameChanged`: a name the server does not have yet.
        if let Some(name) = sent {
            self.send(S::RenameItem(play::RenameItem { name }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Kind, Station};
    use horizon_client::{creative::Names, items::ItemStack};
    use horizon_world::nbt::Tag;

    /// `AnvilScreen.slotChanged`: the box follows the input item, and the name
    /// it shows is what `AnvilScreen.onNameChanged` hands the menu — only a
    /// name the server does not have goes out, and the box's own name is blank.
    /// This is `Station::after_input`, the function `station_after_input` calls
    /// for every slot change.
    #[test]
    fn a_slot_change_sends_only_a_changed_rename() {
        let names = Names::default();
        let mut station = Station::new_kind(Kind::Anvil, [0; 3]);
        station.inputs[0] = Some(ItemStack::new("minecraft:iron_sword", 1));
        assert_eq!(
            station.after_input(&names),
            Some(String::new()),
            "the item's own name goes blank, so the first change is sent"
        );
        assert!(!station.rename.value.is_empty());
        assert!(station.rename.editable);
        assert_eq!(station.after_input(&names), None);

        // A custom input name is preserved, not normalised to blank.
        station.inputs[0].as_mut().unwrap().data_mut().custom_name =
            Some(Tag::String("Sword".into()));
        assert_eq!(station.after_input(&names), Some("Sword".into()));
        assert_eq!(station.rename.value, "Sword");
        assert_eq!(
            station.after_input(&names),
            None,
            "the server already has that name"
        );
        // Emptying the slot leaves the server's menu name intact.
        let input = station.inputs[0].take();
        assert_eq!(station.after_input(&names), None);
        assert_eq!(station.rename.value, "");
        assert!(!station.rename.editable);
        station.inputs[0] = input;
        assert_eq!(station.after_input(&names), None);
        assert_eq!(station.rename.value, "Sword");
        // Editing the box does not change the input stack.
        station.name.clear();
        station.rename.set_value("");
        assert_eq!(
            station.after_input(&names),
            Some(String::new()),
            "blanking it is a change of its own"
        );
        assert_eq!(station.after_input(&names), None);
        // No input item: nothing to rename.
        station.inputs[0] = None;
        assert_eq!(station.after_input(&names), None);
        assert_eq!(station.rename.value, "");
        assert!(!station.rename.editable);
        // And a station that is not the anvil never sends a rename.
        let mut table = Station::new_kind(Kind::Enchanting, [0; 3]);
        table.inputs[0] = Some(ItemStack::new("minecraft:diamond_sword", 1));
        assert_eq!(table.after_input(&names), None);
    }
}
