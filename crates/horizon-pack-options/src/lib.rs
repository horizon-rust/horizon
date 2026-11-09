//! A resource pack's own options (its "configs"), as the game lists, keeps and applies them:
//! each entry's control, default and place on the Pack Options screen, the values a player
//! picked, and the pack-wide presets. The pack formats that declare options (Polytone's
//! `config_entries`, wiki "Polytone Configs") fill these; the engine only stores and edits them.
use std::collections::BTreeMap;

/// A config entry's value: a number (slider), text (cycle button) or boolean (toggle).
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigValue {
    Number(f64),
    Text(String),
    Bool(bool),
}

/// What kind of control a config entry is, with its kind's own fields.
#[derive(Clone, Debug, PartialEq)]
pub enum ConfigKind {
    /// A slider from `min` to `max` in steps of `step`.
    Number { min: f64, max: f64, step: f64 },
    /// A cycle button over `allowed_values`.
    Text { allowed: Vec<String> },
    /// An ON/OFF toggle.
    Bool,
}

/// A config entry: its control, default and how the options screen lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct ConfigEntry {
    pub kind: ConfigKind,
    pub default: ConfigValue,
    /// The `config.<namespace>.section.<id>` group it is listed under, and that group's place.
    pub section: Option<String>,
    pub section_order: Option<i64>,
    pub display_order: i64,
    /// `low`, `medium`, `high` or `varies`.
    pub performance_impact: Option<String>,
    /// A full-width row instead of a half-width one.
    pub wide: bool,
    /// The translation key of the value's label (`%s blocks`).
    pub value_translation: Option<String>,
    /// Its value in each pack-wide preset, and in each preset of its section.
    pub presets: BTreeMap<String, ConfigValue>,
    pub section_presets: BTreeMap<String, ConfigValue>,
}

impl ConfigEntry {
    /// An entry of `kind` starting at `default`, in no section and no preset.
    pub fn new(kind: ConfigKind, default: ConfigValue) -> Self {
        Self {
            kind,
            default,
            section: None,
            section_order: None,
            display_order: 0,
            performance_impact: None,
            wide: false,
            value_translation: None,
            presets: BTreeMap::new(),
            section_presets: BTreeMap::new(),
        }
    }

    /// `value` kept within what the entry allows: a number clamped to its range and snapped
    /// to its step, a text one of its values; `None` for a value of another kind.
    pub fn accept(&self, value: &ConfigValue) -> Option<ConfigValue> {
        match (&self.kind, value) {
            (ConfigKind::Number { min, max, step }, ConfigValue::Number(n)) => {
                let (lo, hi) = (min.min(*max), max.max(*min));
                let mut v = n.clamp(lo, hi);
                if *step > 0.0 {
                    v = (lo + ((v - lo) / step).round() * step).clamp(lo, hi);
                }
                Some(ConfigValue::Number(v))
            }
            (ConfigKind::Text { allowed }, ConfigValue::Text(t)) => {
                (allowed.is_empty() || allowed.contains(t)).then(|| value.clone())
            }
            (ConfigKind::Bool, ConfigValue::Bool(_)) => Some(value.clone()),
            _ => None,
        }
    }
}

/// A pack's config entries and their values, by id (`namespace:name`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Configs {
    values: BTreeMap<String, ConfigValue>,
    entries: BTreeMap<String, ConfigEntry>,
}

impl Configs {
    /// Adds an entry; its value starts at its default.
    pub fn insert(&mut self, id: String, entry: ConfigEntry) {
        self.values.insert(id.clone(), entry.default.clone());
        self.entries.insert(id, entry);
    }

    /// Changes an entry's value (a player's choice), kept within what the entry allows. An id
    /// without an entry takes any value.
    pub fn set(&mut self, id: &str, value: ConfigValue) {
        let value = match self.entries.get(id) {
            Some(entry) => match entry.accept(&value) {
                Some(v) => v,
                None => return,
            },
            None => value,
        };
        self.values.insert(id.to_owned(), value);
    }

    pub fn get(&self, id: &str) -> Option<&ConfigValue> {
        self.values.get(id)
    }

    /// The entries, by id.
    pub fn entries(&self) -> &BTreeMap<String, ConfigEntry> {
        &self.entries
    }

    /// The values that differ from their entries' defaults: what a player chose.
    pub fn chosen(&self) -> BTreeMap<String, ConfigValue> {
        self.values
            .iter()
            .filter(|(id, v)| self.entries.get(*id).is_none_or(|e| &e.default != *v))
            .map(|(id, v)| (id.clone(), v.clone()))
            .collect()
    }

    /// Every entry with a value in `preset` (a pack-wide preset) takes it.
    pub fn apply_preset(&mut self, preset: &str) {
        let picks: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(id, e)| Some((id.clone(), e.presets.get(preset)?.clone())))
            .collect();
        for (id, value) in picks {
            self.set(&id, value);
        }
    }

    /// Takes `other`'s entries and values (another pack's), over its own of the same id.
    pub fn merge(&mut self, other: &Configs) {
        self.entries
            .extend(other.entries.iter().map(|(k, v)| (k.clone(), v.clone())));
        self.values
            .extend(other.values.iter().map(|(k, v)| (k.clone(), v.clone())));
    }

    /// The pack-wide presets' names, in order.
    pub fn presets(&self) -> Vec<String> {
        let mut names: Vec<_> = self
            .entries
            .values()
            .flat_map(|e| e.presets.keys().cloned())
            .collect();
        names.sort();
        names.dedup();
        names
    }
}
