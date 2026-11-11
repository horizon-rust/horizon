//! Polytone's creative tab modifiers (wiki "Creative Tab Modifiers") on the engine's
//! [`crate::pack_hooks::TabModifier`].
use horizon_polytone::creative_tabs::TabModifier;
use std::path::Path;

struct Modifier(TabModifier);

impl crate::pack_hooks::TabModifier for Modifier {
    fn aims_at(&self, tab: &str) -> bool {
        self.0.targets.matches(tab, |_| false)
    }

    fn apply(&self, items: &mut Vec<String>, in_tag: &dyn Fn(&str, &str) -> bool) {
        self.0.apply(items, in_tag);
    }

    fn icon(&self) -> Option<&str> {
        self.0.icon.as_deref()
    }

    fn name(&self) -> Option<&serde_json::Value> {
        self.0.name.as_ref()
    }
}

/// The modifiers of the packs merged at `root`, in load order.
pub fn read(root: &Path) -> Vec<Box<dyn crate::pack_hooks::TabModifier>> {
    TabModifier::read_all(root)
        .into_iter()
        .map(|m| Box::new(Modifier(m)) as _)
        .collect()
}
