//! In-process link adapter for the UNMODIFIED original Horizon game.
//!
//! The iOS host can link this library today. Starting the engine is withheld
//! until its winit event loop and Metal surface can be owned by the iOS host.
//! Do not run Horizon's desktop-style entry function on a UIKit callback.

#[global_allocator]
static ALLOCATOR: horizon_game::Allocator = horizon_game::ALLOCATOR;

/// Keep the official game's entry reachable in the final iOS binary.
/// The game implementation comes from the original upstream crates.
#[unsafe(no_mangle)]
pub extern "C" fn horizon_ios_original_game_entry() {
    horizon_official::register();
    horizon_game::main();
}

/// Used by the same SwiftUI host API as the small Rust stub library.
/// Referencing the official entry forces the linker to retain Horizon code.
#[unsafe(no_mangle)]
pub extern "C" fn horizon_ios_host_api_version() -> u32 {
    std::hint::black_box(horizon_ios_original_game_entry as usize);
    2
}

/// Compilation and native linking do not prove we can run the engine in UIKit.
#[unsafe(no_mangle)]
pub extern "C" fn horizon_ios_game_is_ready() -> bool {
    false
}

#[unsafe(no_mangle)]
pub extern "C" fn horizon_ios_launch_game() -> i32 {
    -1
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn not_playable_until_runtime_and_metal_are_wired() {
        assert_eq!(horizon_ios_host_api_version(), 2);
        assert!(!horizon_ios_game_is_ready());
        assert_eq!(horizon_ios_launch_game(), -1);
    }
}

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

// Weak, non-owning native view handle. The Swift host resets it on teardown.
// No dereference is performed until the engine's real surface adapter exists.
static METAL_VIEW: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
pub extern "C" fn horizon_ios_set_metal_view(view: *mut c_void) {
    METAL_VIEW.store(view as usize, Ordering::Release);
}

#[unsafe(no_mangle)]
pub extern "C" fn horizon_ios_has_metal_view() -> bool {
    METAL_VIEW.load(Ordering::Acquire) != 0
}
