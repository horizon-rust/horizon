//! The official game executable (docs/layers.md, "How a Horizon crate plugs in").
#![forbid(unsafe_code)]

#[global_allocator]
static ALLOCATOR: horizon_game::Allocator = horizon_game::ALLOCATOR;

/// The mod API level this game serves, for the launcher to read without running it; `main`
/// reads it, so the linker keeps it.
static MOD_API_LEVEL: [u8; horizon_game::MOD_API_LEVEL.len()] = horizon_game::MOD_API_LEVEL;

fn main() {
    std::hint::black_box(&MOD_API_LEVEL);
    horizon_official::register();
    // Horizon's mark in the windows and the taskbar (branding: the plain game keeps the default icon).
    horizon_game::set_icon_png(horizon_theme::ICON_PNG);
    horizon_game::main();
}
