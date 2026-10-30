//! What std and a program need on Horizon before and after `main`: the POSIX and thread shims
//! newlib lacks (TLS pointer, `--wrap`ped `threadCreate`, `rename`...), `userAppInit` and
//! `userAppExit`, crash reports, `log.txt` and `panic.txt` beside the NRO, and the heap check
//! that lets the homebrew loader start the next NRO. It is bevy-horizon's runtime without Bevy
//! (docs/switch.md); call [`init`] first thing in `main`.
#![allow(dead_code)]

pub(crate) mod compat;
pub(crate) mod lifecycle;
pub(crate) mod logging;
pub(crate) mod memory;

pub use compat::{
    ThreadCorePolicy, set_thread_core_policy, set_thread_logging, thread_core_policy,
    thread_logging,
};
pub use lifecycle::register_release_hook;
pub use logging::flush_switch_logs;

/// Starts `log.txt` and the panic hook, mounts the RomFS and starts the sockets, and points
/// `HORIZON_HOME` (the game's data folder) at the NRO's folder unless it is set already.
pub fn init(app_name: &str) {
    logging::init_switch_logging(app_name);
    lifecycle::init_services();
    if std::env::var_os("HORIZON_HOME").is_none() {
        if let Some(root) = std::env::var_os("BEVY_ASSET_ROOT") {
            // SAFETY: called first in `main`, before any other thread exists.
            unsafe { std::env::set_var("HORIZON_HOME", root) };
        }
    }
    // The shader cache of NVK and the audio buffers are settled in packaging/switch (docs/switch.md).
}
