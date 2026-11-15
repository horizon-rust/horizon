#!/usr/bin/env python3
"""iOS-only checkout patch: split giant async Renderer::new into two stack frames.

Horizon is upstream-owned. This runs ONLY inside the iOS GitHub Actions
checkout. It changes no gameplay behavior, renderer settings, or GPU work.
UIKit and Metal operations stay on the iOS main thread.

Why: a device crash report showed Renderer::new hitting the main thread's
~1 MB stack guard at its function prologue. The async fn contains one await,
followed by ~2,500 lines of synchronous pipeline setup. The change moves
that synchronous work into a non-inlined ordinary function, so it no longer
resides in the async future's poll stack frame.
"""

from pathlib import Path

ROOT = Path("crates/horizon-client-render/src/gfx.rs")
text = ROOT.read_text(encoding="utf-8")

impl = "impl Renderer {\n    pub async fn new(\n"
assert text.count(impl) == 1, "Renderer::new moved; review upstream before patching"
start = text.index("    pub async fn new(\n", text.index(impl))
signature_end_marker = "    ) -> Result<Self> {\n"
signature_end = text.index(signature_end_marker, start) + len(signature_end_marker)
rest_marker = "        // Computed once: the sprite arrays, the shaders' constants and the"
assert text.count(rest_marker) == 1, "Renderer device-setup boundary changed"
rest_start = text.index(rest_marker, signature_end)
end_marker = "\n    /// The first-person hand's quads"
assert text.count(end_marker) == 1, "Renderer::new end marker changed"
end = text.index(end_marker, rest_start)

signature = text[start:signature_end]
before = text[signature_end:rest_start]
after = text[rest_start:end]
opening = """        let Opened {
            instance,
            surface,
            adapter,
            device,
            queue,
            device_lost,
            configured,
        } = match previous {"""
assert before.count(opening) == 1, "Renderer opened-device match changed"
assert before.count(".await") == 1 and ".await" not in after, (
    "Renderer::new has new await points; cannot split automatically"
)
assert after.rstrip().endswith("    }"), "Renderer::new closing brace changed"
assert 'startup_step(format_args!("renderer.new.entry"))' in before, (
    "Apply the Nova renderer diagnostics patch before stack splitting"
)

args = (
    "window", "offscreen", "sprite_pixels", "sprite_size",
    "sprite_minimum_size", "layers", "sprite_mips", "animations",
    "hud_atlas", "glint", "glint_armor", "forcefield", "panorama_faces",
    "prepared", "previous", "api", "translucent_batching", "shader_pack",
    "settings", "request", "display",
)
for name in args:
    assert f"        {name}:" in signature, f"Renderer argument {name} changed"

# Keep the original async API and the exact device open/reuse logic.
before = before.replace(opening, "        let opened = match previous {", 1)
call = "\n".join(f"            {name}," for name in args)
async_version = (
    signature
    + before
    + '        startup_step(format_args!("renderer.async.device-opened"));\n'
    + "        Self::nova_ios_finish_new(\n"
    + "            opened,\n"
    + call
    + "\n        )\n"
    + "    }"
)

# The huge, synchronous renderer setup gets its own physical stack frame.
helper_sig = signature.replace(
    "    pub async fn new(\n",
    "    fn nova_ios_finish_new(\n        opened: Opened,\n",
    1,
).replace("        api: GraphicsApi,\n", "        _api: GraphicsApi,\n", 1)
destructure = opening.replace(" = match previous {", " = opened;")
helper = (
    "\n    /// iOS runner-only stack isolation for the synchronous GPU pipeline setup.\n"
    + "    #[inline(never)]\n"
    + helper_sig
    + '        startup_step(format_args!("renderer.finish.enter"));\n'
    + destructure
    + "\n"
    + after
)

updated = text[:start] + async_version + helper + text[end:]
assert updated.count("fn nova_ios_finish_new(") == 1
assert updated.count("pub async fn new(") >= 1
assert len(updated) > len(text)
ROOT.write_text(updated, encoding="utf-8")
print(
    "Nova iOS: split Renderer::new at its single await into an async "
    "device-open stage and a non-inlined synchronous initialization stage"
)
