#!/usr/bin/env python3
"""Apply Kelly-Inc/horizon-ios's winit callback stack-growth pattern on iOS.

This is an ephemeral CI patch to upstream Horizon. It does not change
original tracked game sources, gameplay, or the UIKit/main-thread owner.

Kelly-Inc/horizon-ios uses stacker::maybe_grow(1 << 20, 32 << 20, work)
around the event callbacks. A physical Nova iOS crash on 2026-11-15
hit a 16KB guard below its 1008KB main-thread stack, at Renderer::new
from the resumed callback. The crucial difference from moving a future
to a spawned thread is that stacker keeps execution on the same
OS thread while switching to a bigger stack for the callback.
"""

from pathlib import Path

manifest = Path("crates/horizon-client/Cargo.toml")
before = manifest.read_text(encoding="utf-8")
anchor = 'pollster = "0.4"\n'
assert before.count(anchor) == 1, "Upstream manifest dependencies changed"
assert "\nstacker = " not in before, "Upstream already uses stacker; reassess patch"
manifest.write_text(
    before.replace(
        anchor,
        anchor
        + '# Nova iOS: grow stack for winit callbacks, as Kelly-Inc/horizon-ios.\n'
        + 'stacker = "=0.1.25"\n',
        1,
    ),
    encoding="utf-8",
)

path = Path("crates/horizon-client/src/game/loading_screen.rs")
source = path.read_text(encoding="utf-8")
start_token = "impl ApplicationHandler for App {\n"
end_token = "\n}\n\n/// Every launch reads the prepared game files"
assert source.count(start_token) == 1 and source.count(end_token) == 1, (
    "Upstream App handler structure changed; review before patching"
)
start = source.index(start_token)
end = source.index(end_token, start) + len("\n}")
original = source[start:end]
assert original.count("    fn resumed(") == 1
assert original.count("    fn window_event(") == 1
assert original.count("    fn device_event(") == 1
assert original.count("    fn about_to_wait(") == 1
assert original.count("    fn exiting(") == 1

helpers = original[len(start_token):-len("\n}")]
for name in ("resumed", "window_event", "device_event", "about_to_wait", "exiting"):
    helpers = helpers.replace(f"    fn {name}(", f"    fn {name}_here(", 1)

replacement = '''// Nova iOS-only adaptation of Kelly-Inc/horizon-ios's stacker::maybe_grow.
// The iOS main thread is ~1 MiB and renderer initialization overflows it.
// Unlike std::thread::spawn, this keeps UIKit and Metal on the same OS thread.
#[inline(never)]
fn nova_ios_deep<T>(work: impl FnOnce() -> T) -> T {
    #[cfg(target_os = "ios")]
    {
        stacker::maybe_grow(1 << 20, 32 << 20, work)
    }
    #[cfg(not(target_os = "ios"))]
    {
        work()
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(target_os = "ios")]
        nova_diag("ios.deep-stack.resumed.begin");
        nova_ios_deep(|| self.resumed_here(event_loop));
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        nova_ios_deep(|| self.window_event_here(event_loop, id, event));
    }
    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        nova_ios_deep(|| self.device_event_here(event_loop, id, event));
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        nova_ios_deep(|| self.about_to_wait_here(event_loop));
    }
    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        nova_ios_deep(|| self.exiting_here(event_loop));
    }
}

impl App {
''' + helpers + '\n}'

assert source[start:end] == original
updated = source[:start] + replacement + source[end:]
assert updated.count("fn nova_ios_deep<") == 1
assert updated.count("fn resumed_here(") == 1
assert updated.count("stacker::maybe_grow(1 << 20, 32 << 20, work)") == 1
path.write_text(updated, encoding="utf-8")
print("Nova iOS: wrapped all five winit ApplicationHandler callbacks in a")
print("same-thread 32 MiB stacker scope, preserving the original bodies.")
