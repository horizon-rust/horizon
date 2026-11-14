//! A link probe for the real, unmodified Horizon game, not a gameplay fork.
//!
//! Do NOT call this entry from UIKit/SwiftUI yet. Horizon's existing game
//! startup owns the event loop and process arguments, and iOS window/event
//! loop hand-off has not been implemented. This library verifies whether
//! the original Horizon game code can be built as part of a native iOS app.

/// Same upstream initialization order as horizon-official/src/main.rs.
///
/// This symbol is intentionally not linked into the user-facing host yet.
/// A later milestone must supply a correct iOS lifecycle and window surface.
#[unsafe(no_mangle)]
pub extern "C" fn horizon_ios_original_game_entry() {
    horizon_official::register();
    horizon_game::main();
}
