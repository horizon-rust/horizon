//! This repository's plain executable: the platform's game with no Horizon part (docs/layers.md).
//! The official executable is horizon-official's.
#![forbid(unsafe_code)]

#[cfg(not(feature = "system-allocator"))]
#[global_allocator]
static ALLOCATOR: horizon_threads::GameAllocator = horizon_threads::GAME_ALLOCATOR;

/// The mod API level this game serves, kept in the program for the launcher
/// to read without running it (`horizon_mods::api_level::MARK`). `main`
/// reads it, so the linker keeps it.
static MOD_API_LEVEL: [u8; horizon_game::MOD_API_LEVEL.len()] = horizon_game::MOD_API_LEVEL;

fn main() {
    std::hint::black_box(&MOD_API_LEVEL);
    horizon_game::main();
}
