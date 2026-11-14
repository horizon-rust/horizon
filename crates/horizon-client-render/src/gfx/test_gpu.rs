//! The GPU tests' `wgpu::Instance`, and their turns on the GPU.

use std::cell::OnceCell;
use std::sync::{Condvar, Mutex, PoisonError};

/// Whether a test thread is on the GPU, and the wait for it to end.
static BUSY: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

/// One GPU test's turn, given back when its thread ends.
struct Turn;

impl Drop for Turn {
    fn drop(&mut self) {
        *BUSY.0.lock().unwrap_or_else(PoisonError::into_inner) = false;
        BUSY.1.notify_one();
    }
}

thread_local!(static TURN: OnceCell<Turn> = const { OnceCell::new() });

/// On Windows, waits until no other test thread is on the GPU; the thread keeps its turn
/// until it ends. NVIDIA's DirectX 12 driver deadlocks when one thread creates a device
/// while others use theirs (`D3D12CreateDevice` waits in `nvwgf2umx.dll` for a lock the
/// others wait on too), and the suite hung most runs, with 4 GPU tests at once as with 12
/// (#16295). [`instance`] calls it; a test that makes its own instance calls it first.
pub(crate) fn turn() {
    if !cfg!(windows) {
        return;
    }
    TURN.with(|turn| {
        turn.get_or_init(|| {
            let mut busy = BUSY.0.lock().unwrap_or_else(PoisonError::into_inner);
            while *busy {
                busy = BUSY.1.wait(busy).unwrap_or_else(PoisonError::into_inner);
            }
            *busy = true;
            Turn
        });
    });
}

/// A new instance for one GPU test. On Windows it opens Vulkan and DirectX 12,
/// not OpenGL: WGL instances made and dropped from many test threads at once
/// deadlocked NVIDIA's driver, and the render tests hung most runs (#5238).
/// The game runs there on Vulkan or DirectX 12 unless neither works.
/// `WGPU_BACKEND` and the other `WGPU_*` variables still apply, as in the game.
pub(crate) fn instance() -> wgpu::Instance {
    turn();
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    if cfg!(windows) {
        descriptor.backends = wgpu::Backends::PRIMARY;
    }
    wgpu::Instance::new(descriptor.with_env())
}
