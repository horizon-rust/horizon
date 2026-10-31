//! Namespaced registries, mirroring Fabric's `Registry` surface.

use std::collections::HashMap;
use std::fmt;
use thiserror::Error;

/// A namespaced identifier (`namespace:path`), as used by Minecraft and Fabric.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Identifier {
    /// Namespace (defaults to `minecraft`).
    pub namespace: String,
    /// Path within the namespace.
    pub path: String,
}

impl Identifier {
    /// The default namespace.
    pub const DEFAULT_NAMESPACE: &'static str = "minecraft";

    /// Create an identifier, validating both parts.
    pub fn new(namespace: impl Into<String>, path: impl Into<String>) -> Result<Self, RegistryError> {
        let namespace = namespace.into();
        let path = path.into();
        if !is_valid_part(&namespace) {
            return Err(RegistryError::InvalidIdentifier(format!(
                "invalid namespace `{namespace}`"
            )));
        }
        if !is_valid_part(&path) {
            return Err(RegistryError::InvalidIdentifier(format!(
                "invalid path `{path}`"
            )));
        }
        Ok(Self { namespace, path })
    }

    /// Parse `namespace:path`, defaulting the namespace to `minecraft`.
    pub fn parse(value: &str) -> Result<Self, RegistryError> {
        match value.split_once(':') {
            Some((namespace, path)) => Self::new(namespace, path),
            None => Self::new(Self::DEFAULT_NAMESPACE, value),
        }
    }

    /// The `namespace:path` form.
    pub fn as_string(&self) -> String {
        format!("{}:{}", self.namespace, self.path)
    }
}

impl fmt::Display for Identifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.namespace, self.path)
    }
}

impl std::str::FromStr for Identifier {
    type Err = RegistryError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

fn is_valid_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.' | '/'))
}

/// Errors from registry operations.
#[derive(Debug, Error)]
pub enum RegistryError {
    /// The identifier is malformed.
    #[error("invalid identifier: {0}")]
    InvalidIdentifier(String),
    /// The identifier is already registered.
    #[error("`{0}` is already registered")]
    Duplicate(String),
    /// The identifier is not registered.
    #[error("`{0}` is not registered")]
    NotFound(String),
}

/// A registry of values keyed by [`Identifier`].
#[derive(Debug, Clone)]
pub struct Registry<T> {
    name: &'static str,
    entries: HashMap<Identifier, T>,
}

impl<T> Registry<T> {
    /// Create an empty registry.
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            entries: HashMap::new(),
        }
    }

    /// The registry's name, used in diagnostics.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// Register a value, failing if the identifier is taken.
    pub fn register(&mut self, id: Identifier, value: T) -> Result<(), RegistryError> {
        if self.entries.contains_key(&id) {
            return Err(RegistryError::Duplicate(id.to_string()));
        }
        self.entries.insert(id, value);
        Ok(())
    }

    /// Get a value by identifier.
    pub fn get(&self, id: &Identifier) -> Option<&T> {
        self.entries.get(id)
    }

    /// Whether an identifier is registered.
    pub fn contains(&self, id: &Identifier) -> bool {
        self.entries.contains_key(id)
    }

    /// Iterate over every entry.
    pub fn iter(&self) -> impl Iterator<Item = (&Identifier, &T)> {
        self.entries.iter()
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// An item registered by a mod.
#[derive(Debug, Clone, Default)]
pub struct ItemEntry {
    /// The item's identifier.
    pub id: String,
    /// Stack size.
    pub max_count: u32,
    /// Durability, if damageable.
    pub max_damage: Option<u32>,
    /// Whether the item survives fire.
    pub fire_resistant: bool,
}

/// A block registered by a mod.
#[derive(Debug, Clone, Default)]
pub struct BlockEntry {
    /// The block's identifier.
    pub id: String,
    /// Hardness.
    pub hardness: f32,
    /// Blast resistance.
    pub resistance: f32,
    /// Light emitted, 0..=15.
    pub light_level: u8,
}

/// An entity type registered by a mod.
#[derive(Debug, Clone, Default)]
pub struct EntityTypeEntry {
    /// The entity type's identifier.
    pub id: String,
    /// Collision width.
    pub width: f32,
    /// Collision height.
    pub height: f32,
    /// Whether the entity is fire immune.
    pub fire_immune: bool,
}
