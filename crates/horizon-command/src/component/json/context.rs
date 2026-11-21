//! Borrowed registry authority for metadata's context-free component codec.
use super::value::{Map, Value};

/// Borrows the loaded catalog without promoting plain JsonOps into RegistryOps.
/// Dynamic holder references therefore retain their context-free semantics.
pub struct MetadataContext<'a> {
    pub registries: &'a Registries,
}
impl Context for MetadataContext<'_> {
    fn item_template(&self, value: &Map<String, Value>) -> Result<(), Error> {
        super::item::validate(value, self.registries, |id, value| {
            super::payload::validate(id, value, self)
        })
    }
    fn entity_type(&self, id: &str) -> Result<(), Error> {
        registered(&self.registries.entity_types, id)
    }
    fn encode_item_template(&self, value: &Map<String, Value>) -> Result<Value, Error> {
        super::canonical::item(value, self)
    }
}
pub(super) fn registered(ids: &[String], id: &str) -> Result<(), Error> {
    if ids.is_empty() {
        return Err(Error::Unavailable);
    }
    super::require(ids.iter().any(|known| known == id))
}
use super::{Context, Error};
use crate::Registries;

/// The nested codecs, operating on JSON values with JsonOps semantics. These
/// are required operations, not permissive defaults or a text-rendering hook.
pub trait PayloadCodecs {
    fn component(&self, id: &str, value: &Value, context: &dyn Context) -> Result<(), Error>;
}

/// A view of the registry snapshot already loaded by the game. Constructing
/// or querying this view performs no I/O and stores no second registry.
pub struct BorrowedContext<'a> {
    pub registries: &'a Registries,
    pub payloads: &'a dyn PayloadCodecs,
}
impl Context for BorrowedContext<'_> {
    fn item_template(&self, value: &Map<String, Value>) -> Result<(), Error> {
        super::item::validate(value, self.registries, |id, value| {
            self.payloads.component(id, value, self)
        })
    }
    fn entity_type(&self, id: &str) -> Result<(), Error> {
        let ids = &self.registries.entity_types;
        if ids.is_empty() {
            return Err(Error::Unavailable);
        }
        if ids.iter().any(|known| known == id) {
            Ok(())
        } else {
            Err(Error::Invalid)
        }
    }
}

/// A context with no registry access. Pure text still has a complete codec;
/// registry-bearing/nested payloads cannot be guessed from their spelling.
pub struct UnavailableContext;
impl Context for UnavailableContext {
    fn item_template(&self, _: &Map<String, Value>) -> Result<(), Error> {
        Err(Error::Unavailable)
    }
    fn entity_type(&self, _: &str) -> Result<(), Error> {
        Err(Error::Unavailable)
    }
}
