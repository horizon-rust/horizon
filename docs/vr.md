# VR (experimental)

Horizon draws to a headset through OpenXR, on any runtime (SteamVR, Monado, Meta, Virtual Desktop...). It is a
research spike: off unless the game starts with `HORIZON_XR=1`, Vulkan only, and the OpenXR loader
(`openxr_loader.dll` on Windows, from the Khronos OpenXR-SDK releases) beside the game. Without the loader or a
headset the game logs why and starts as usual.

| Part | Where |
|---|---|
| OpenXR session, Vulkan through the runtime, swapchains, frame loop, controllers (actions) | `crates/horizon-xr` |
| The eyes, the HUD panel, head aim, the controllers as a gamepad | `crates/horizon-client/src/gfx/xr.rs`, `game/pad.rs` (`xr_look`, `poll_pads`) |
| A fake runtime for tests without a headset | `crates/horizon-xr-fake`, `tools/vr_fake_run.py` |
| wgpu-core reading the environment's flags for an instance made from wgpu-hal | `third_party/wgpu-core` (`src/instance.rs`) |

## How a frame is drawn

1. `xrWaitFrame` paces the game; the head and both eyes are located in the stage space.
2. Each eye draws the world through the renderer's offscreen path into its swapchain image, at the headset's size,
   with its off-axis projection. The body's yaw is the player's less the head's, and the head's pose goes on top.
   The sky folds each eye's off-centre frustum in, so both eyes see the same sky.
3. Both eyes cull with one frustum around them; the second eye reuses the first one's visible sections and draw
   lists (GPU occlusion culling is off in the eyes, it reads each eye's depth). The world post (resolution scale,
   FXAA) is skipped in the eyes.
4. The window shows the left eye and the HUD (no third world draw).
5. The HUD and menus go onto a panel (an OpenXR quad layer) 1.5 m ahead, which follows the head once it turns
   40 degrees away; it is drawn again only when the HUD changed.

The head aims (`xr_look` turns the player by the head's yaw and sets its pitch). Controllers come in as one more
gamepad: sticks, triggers, grips, A/B/X/Y, stick clicks and Menu, bound for Touch, Index and the simple controller.

## Testing without a headset

```sh
cargo build --release -p horizon-client -p horizon-xr -p horizon-xr-fake
python3 tools/vr_fake_run.py --resources GAME_FOLDER_WITH_.local --out vr-run
python3 tools/vr_fake_run.py --resources ... --settings-from GAME_FOLDER --size 3132x3016 --hz 1000 \
    --world-frames 9000 --script tools/vr_fake_scripts/look_around.txt
```

The game runs from a temporary folder (the prepared resources linked read-only, a fresh world), on the fake
runtime through `XR_RUNTIME_JSON` for that process only. `vr-run/report.json` has the frame count, frame times
(all and the last 300 frames), the layers submitted and the actions created; `latest-*.png` are each eye and the
panel. `--hz 1000` never waits, to measure the game; `HORIZON_SPIKES=0` in the environment logs every frame's
spans (`vr_eyes`, `vr_eye_draw`, `draw_lists`, `passes`, `encode`, `vr_panel`...). The fake runtime's settings
and script format are in `crates/horizon-xr-fake/src/lib.rs`.

## Measured (fake runtime, RTX 4090, 3132x3016 per eye, render distance 64, distant terrain 1024)

Frames a second once the world had loaded, unpaced:

| Step | fps | ms |
|---|---|---|
| First stereo version (eyes, window and panel each a full draw, FXAA per eye) | 33 while loading | - |
| No world post in the eyes, window copies the left eye, one cull for both eyes | 61 | 16.3 |
| The second eye reuses the first eye's draw lists | 81 | 12.3 |
| No indirect draw validation on OpenXR's instance (`third_party/wgpu-core`) | 102 | 9.8 |
| Distant terrain finds tiles to remesh without testing every pair | 106 | 9.4 |

What is left per frame (both eyes): recording the passes (1.9 ms), `encode` (1.7 ms), draw lists (1.35 ms),
visibility (0.7 ms); on the main thread, `update_world` (2.2 ms) and distant terrain (1.5 ms). Single-pass stereo
(multiview) would halve the passes and `encode`.

## Not there yet

Room-scale movement, aiming and pointing with a controller, the hand and held item, the hotbar on the off hand,
snap turning, and an antialiasing that suits VR (MSAA with alpha to coverage).
