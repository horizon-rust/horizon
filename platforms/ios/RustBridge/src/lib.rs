//! iOS host ABI scaffold. Deliberately does not substitute or alter gameplay.
//! When Horizon's iOS runtime integration is proven, wire these entry points
//! into the ORIGINAL upstream Horizon client, not a copied game implementation.

#[no_mangle]
pub extern "C" fn horizon_ios_host_api_version() -> u32 {
    1
}

#[no_mangle]
pub extern "C" fn horizon_ios_game_is_ready() -> bool {
    false
}

/// Returns an explicit not-integrated error instead of claiming a game launch.
#[no_mangle]
pub extern "C" fn horizon_ios_launch_game() -> i32 {
    -1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_status_is_explicit() {
        assert_eq!(horizon_ios_host_api_version(), 1);
        assert!(!horizon_ios_game_is_ready());
        assert_eq!(horizon_ios_launch_game(), -1);
    }
}

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

// Weak, non-owning native view handle. The Swift host resets it on teardown.
// No dereference is performed until the engine's real surface adapter exists.
static METAL_VIEW: AtomicUsize = AtomicUsize::new(0);

#[no_mangle]
pub extern "C" fn horizon_ios_set_metal_view(view: *mut c_void) {
    METAL_VIEW.store(view as usize, Ordering::Release);
}

#[no_mangle]
pub extern "C" fn horizon_ios_has_metal_view() -> bool {
    METAL_VIEW.load(Ordering::Acquire) != 0
}

#[cfg(test)]
mod metal_tests {
    #[test]
    fn metal_view_starts_unattached() {
        assert!(!super::horizon_ios_has_metal_view());
    }
}

#[path = "../../AssetImporter/src/ffi_adapter.rs"]
mod nova_asset_ffi;

/// Lightweight launcher builds cannot start the game; return control to SwiftUI.
#[no_mangle]
pub extern "C" fn horizon_ios_start_original_game(
    _resources: *const std::ffi::c_char,
    _home: *const std::ffi::c_char,
) -> i32 {
    -1
}
