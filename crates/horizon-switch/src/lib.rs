//! Switch input for Horizon: libnx's HID turned into the plain `Frame` the game's pad code reads
//! (`horizon-client/src/game/pad/hardware.rs`). The mapping, hotplug and touch logic are pure and
//! tested on any host; only `hid` talks to libnx and exists on `target_os = "horizon"`.
#[cfg(any(target_os = "horizon", feature = "ffi"))]
mod hid;
mod npad;
#[cfg(any(target_os = "horizon", feature = "rt-check"))]
pub mod rt;
#[cfg(any(target_os = "horizon", feature = "ffi"))]
pub mod swkbd;
mod touch;

#[cfg(any(target_os = "horizon", feature = "ffi"))]
pub use hid::{Hid, Libnx, TouchScreen};
pub use npad::{Frame, Key, Raw, Tracker, bits, style};
pub use touch::{Pointer, Touch, Tracker as TouchTracker};
