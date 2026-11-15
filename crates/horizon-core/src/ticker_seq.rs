//! When a block entity's ticker joined `Level.blockEntityTickers`, as one
//! number that rises with every join, whatever the kind.
//!
//! Vanilla keeps every ticker in one list and ticks it in join order. Horizon
//! keeps moving pistons (`Ticks::moving_pistons`) apart from the other block
//! entities, so a pass that must follow vanilla's order across both (a geyser's
//! push and a piston's push of the same tick) compares these numbers.
//! One counter for the process, so two levels' numbers compare too; only
//! their order means anything.

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

/// A number above every one handed out before.
pub fn next() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}
