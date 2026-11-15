//! In-process link adapter for the UNMODIFIED original Horizon game.
//!
//! The iOS host can link this library today. Starting the engine is withheld
//! until its winit event loop and Metal surface can be owned by the iOS host.
//! Do not run Horizon's desktop-style entry function on a UIKit callback.

mod diagnostics;

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

#[path = "../../AssetImporter/src/ffi_adapter.rs"]
mod nova_asset_ffi;

/// This MUST be called once, from the main thread, before UIKit has started.
/// The original Horizon client's winit event loop owns UIApplicationMain.
/// Returns an error only if required private directories are unavailable or
/// if the game exits its application loop unexpectedly.
///
/// This integration route is an on-device experiment until native startup,
/// lifecycle events, audio, and Metal are verified.
#[unsafe(no_mangle)]
pub extern "C" fn horizon_ios_start_original_game(
    resources: *const std::ffi::c_char,
    home: *const std::ffi::c_char,
) -> i32 {
    use std::{env, ffi::CStr, path::PathBuf};

    fn parse(ptr: *const std::ffi::c_char) -> Option<PathBuf> {
        if ptr.is_null() {
            return None;
        }
        // SAFETY: Nova's Swift bridge sends a valid NUL-terminated C string
        // alive for the complete duration of this call.
        let text = unsafe { CStr::from_ptr(ptr) }.to_str().ok()?;
        if text.is_empty() {
            return None;
        }
        Some(PathBuf::from(text))
    }

    diagnostics::trace("rust.start.entry");
    let (Some(resources), Some(home)) = (parse(resources), parse(home)) else {
        diagnostics::trace("rust.start.invalid-paths");
        return -2;
    };
    if !resources.join("horizon-resources.json").is_file()
        || !resources.join("assets/minecraft").is_dir() {
        diagnostics::trace("rust.start.missing-resources");
        return -3;
    }
    diagnostics::trace("rust.start.resources-present");
    if std::fs::create_dir_all(&home).is_err()
        || env::set_current_dir(&home).is_err() {
        diagnostics::trace("rust.start.game-home-unavailable");
        return -4;
    }
    diagnostics::trace("rust.start.game-home-ready");

    // SAFETY: the process is in its single-threaded startup stage: before the
    // game's workers, SwiftUI or the winit UIApplication delegate are created.
    unsafe {
        env::set_var("NOVA_IOS_RESOURCES", &resources);
        env::set_var("HORIZON_HOME", &home);
    }
    // Continue with the ORIGINAL official Horizon startup and its winit loop.
    // This function normally never returns on iOS.
    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // An unexpected Rust panic may terminate the iOS process before
        // the renderer reports a normal error.
        diagnostics::trace(&format!("rust.panic: {info}"));
        previous_hook(info);
    }));
    diagnostics::trace("rust.start.horizon-official-entry");
    horizon_ios_original_game_entry();
    diagnostics::trace("rust.start.horizon-returned");
    0
}

/// Send virtual key, mouse, pointer and text events to Horizon's normal input
/// mappings. Enqueued from UIKit; consumed on the game's event-loop thread.
#[unsafe(no_mangle)]
pub extern "C" fn nova_ios_input_key(code: u32, down: bool) {
    horizon_client_input::ios_input::push(
        horizon_client_input::ios_input::Input::Key(code, down)
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn nova_ios_input_mouse(button: u32, down: bool) {
    horizon_client_input::ios_input::push(
        horizon_client_input::ios_input::Input::Mouse(button, down)
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn nova_ios_input_cursor(x: f64, y: f64) {
    horizon_client_input::ios_input::push(
        horizon_client_input::ios_input::Input::Cursor(x, y)
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn nova_ios_input_motion(x: f64, y: f64) {
    horizon_client_input::ios_input::push(
        horizon_client_input::ios_input::Input::Motion(x, y)
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn nova_ios_input_text(text: *const std::ffi::c_char) {
    if text.is_null() { return }
    // SAFETY: UIKit passes a live NUL-terminated UTF-8 string.
    if let Ok(text) = unsafe { std::ffi::CStr::from_ptr(text) }.to_str() {
        let clipped = text.chars().take(512).collect::<String>();
        horizon_client_input::ios_input::push(
            horizon_client_input::ios_input::Input::Text(clipped)
        );
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn nova_ios_input_close() {
    horizon_client_input::ios_input::push(
        horizon_client_input::ios_input::Input::Close
    );
}

#[unsafe(no_mangle)]
pub extern "C" fn nova_ios_set_touch_gestures(enabled: bool) {
    horizon_client_input::ios_input::set_touch_gestures(enabled);
}

#[unsafe(no_mangle)]
pub extern "C" fn nova_ios_set_overlay_active(active: bool) {
    horizon_client_input::ios_input::set_overlay_active(active);
}
