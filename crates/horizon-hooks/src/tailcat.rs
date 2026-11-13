//! Where the Tailcat helper is downloaded from: a Horizon crate's address, handed to the engine
//! before the game starts ([`set_assets`]). The engine keeps the download and the checksum; this
//! static only lets the Horizon layer name the address (docs/layers.md#built-in-addresses).

use std::sync::OnceLock;

static ASSETS: OnceLock<&'static str> = OnceLock::new();

/// Names the helper's download base (`crates/horizon-addresses`). The first call wins: a later
/// one is a second registration, not a change.
pub fn set_assets(url: &'static str) {
    let _ = ASSETS.set(url);
}

/// The base URL, once a Horizon crate set one. Without one (a plain build, the server build) the
/// helper can still be found on disk or in `HORIZON_TAILCAT_HELPER`, but not downloaded.
pub fn assets() -> Option<&'static str> {
    ASSETS.get().copied()
}
