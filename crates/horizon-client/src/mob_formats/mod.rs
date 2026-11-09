//! The pack formats vanilla doesn't read that shape mobs (OptiFine's and Entity Model Features'
//! custom entity models) on the engine's extension point ([`crate::mobs::cem::install`]). Platform
//! code (docs/layers.md) kept in this crate until a platform crate installs it: it uses only the
//! crate's public API, and nothing else in the crate names it.

pub mod cem;
mod cem_anim;

use crate::mobs::MobType;
use crate::mobs::cem::{CemModels, ModelFormats};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

/// OptiFine's custom entity models.
pub struct Formats;

impl ModelFormats for Formats {
    fn entity_models(
        &self,
        root: &Path,
        types: &HashMap<String, Arc<MobType>>,
    ) -> (CemModels, Vec<String>) {
        cem::load(root, types)
    }
}

/// Installs these formats ([`crate::mobs::cem::install`]); the game does at startup.
pub fn install() {
    crate::mobs::cem::install(&Formats);
}
