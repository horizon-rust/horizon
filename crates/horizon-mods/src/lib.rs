//! Horizon's mod files (`docs/mods.md`): content mods (`.hmod`: blocks, items, data and
//! assets, no code), modpacks and, with `index`, the mod registry's index. Code mods are
//! native (docs/native-mods.md).
pub mod api_level;
pub mod compat;
pub mod content;
#[cfg(feature = "index")]
pub mod index;
pub mod manifest;
pub mod modpack;

pub use compat::{ModLoaderType, ProfileModsReport, inspect_mod, scan_mods_dir};
pub use content::Content;
pub use manifest::{Capability, Manifest, Package, Side, find};
