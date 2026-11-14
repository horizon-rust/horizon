//! `AnvilScreen`'s rename box: the 50-character `EditBox`, read-only while
//! the input slot is empty (`AnvilScreen.slotChanged`).
use super::{Kind, Station};
use horizon_client::{creative::Names, stations::hover_name};

/// `AnvilScreen`: `name.setMaxLength(50)`.
pub(super) const NAME_CHARS: usize = 50;

/// `AnvilScreen.init`: the unbordered box of 50 at the menu's name position,
/// focused; `slotChanged` makes it editable once the input slot has an item.
pub(super) fn rename_box() -> horizon_client::ui::EditBox {
    let mut edit = horizon_client::ui::EditBox::new(0.0, 0.0, 103.0, 12.0);
    edit.message = super::super::screen_info::tr("container.repair", &[]);
    edit.bordered = false;
    edit.max_length = NAME_CHARS;
    edit.text_color = u32::MAX;
    // AnvilScreen also sets `setTextColorUneditable(-1)`: white either way.
    edit.text_color_uneditable = u32::MAX;
    // AnvilScreen sets `setInvertHighlightedTextColor(false)`.
    edit.invert_highlighted_text_color = false;
    edit.focused = true;
    edit.editable = false;
    edit
}

impl Station {
    /// The anvil name the menu works with (`AnvilScreen.onNameChanged`).
    pub(in crate::game) fn anvil_item_name(&self, names: &Names) -> String {
        match &self.inputs[0] {
            Some(input)
                if input.custom_name().is_none()
                    && self.name == names.display_item(&input.item) =>
            {
                String::new()
            }
            Some(_) => self.name.clone(),
            None => String::new(),
        }
    }

    /// `AnvilScreen.onNameChanged`: a name the anvil menu takes goes to the
    /// server only if it differs from the one last sent. `name` is blank when
    /// the box shows the input item's own name.
    pub(in crate::game) fn take_anvil_rename(&mut self, name: String) -> Option<String> {
        if self.anvil_sent.as_deref() == Some(name.as_str()) {
            return None;
        }
        self.anvil_sent = Some(name.clone());
        Some(name)
    }
}

impl Station {
    /// `AnvilScreen.slotChanged` as the game runs it: the box follows the input
    /// item, then `onNameChanged` normalises the text for the menu. `None` when
    /// this is not an anvil, the input slot is empty, or the normalised name
    /// is the one already sent (the menu's `itemName`).
    pub(in crate::game) fn after_input(&mut self, names: &Names) -> Option<String> {
        if self.kind != Kind::Anvil {
            return None;
        }
        if self.inputs[0] != self.anvil_last {
            self.anvil_last = self.inputs[0].clone();
            let name = self.inputs[0]
                .as_ref()
                .map(|i| hover_name(names, i))
                .unwrap_or_default();
            input_changed(self, name);
        }
        self.inputs[0].as_ref()?;
        let name = self.anvil_item_name(names);
        self.take_anvil_rename(name)
    }
}

/// `AnvilScreen.slotChanged`: the box shows the input's name, and only an
/// input item makes it editable (`name.setEditable(!itemStack.isEmpty())`).
/// A read-only box still keeps the focus, the caret keys and copy.
pub(super) fn input_changed(station: &mut Station, name: String) {
    station.name = name;
    let value = station.name.clone();
    station.rename.set_value(&value);
    station.rename.editable = station.inputs[0].is_some();
}

#[cfg(test)]
mod tests {
    use super::super::{Kind, Station};
    use super::{input_changed, rename_box};

    #[test]
    fn the_rename_box_is_read_only_without_an_input_item() {
        // AnvilScreen.init: no input yet, so the box is read-only but focused.
        let station_box = rename_box();
        assert!(station_box.focused && !station_box.editable);
        assert!(!station_box.invert_highlighted_text_color);

        // slotChanged with an item: editable, and it shows the item's name.
        let mut station = Station::new_kind(Kind::Anvil, [0; 3]);
        station.inputs[0] = Some(crate::items::ItemStack::new("minecraft:diamond_sword", 1));
        input_changed(&mut station, "Diamond Sword".into());
        assert!(station.rename.editable);
        assert_eq!(station.rename.value, "Diamond Sword");

        // The input is taken out again: read-only, and slotChanged's
        // `setValue("")` empties it (the caller passes the empty name).
        station.inputs[0] = None;
        input_changed(&mut station, String::new());
        assert!(!station.rename.editable);
        assert_eq!(station.rename.value, "");
        station.rename.set_value("Diamond Sword");

        // A read-only box still keeps the caret keys; Backspace is consumed
        // without taking a character.
        assert!(
            station
                .rename
                .key(winit::keyboard::KeyCode::ArrowLeft, false, false)
        );
        assert!(
            station
                .rename
                .key(winit::keyboard::KeyCode::Backspace, false, false)
        );
        assert_eq!(station.rename.value, "Diamond Sword");
    }
}

// The previews in a server session read the server's enchantments (#8688).
#[cfg(test)]
#[path = "remote_anvil_tests.rs"]
mod remote_anvil_tests;
