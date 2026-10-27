# VR (experimental)

Horizon draws to a headset through OpenXR, on any runtime (SteamVR, Monado, Meta, Virtual Desktop...). It is a
research spike: off unless the game starts with `HORIZON_XR=1`, Vulkan only, and the OpenXR loader
(`openxr_loader.dll` on Windows, from the Khronos OpenXR-SDK releases) beside the game. Without the loader or a
headset the game logs why and starts as usual.

| Part | Where |
|---|---|
| OpenXR session, Vulkan through the runtime, swapchains, frame loop, controllers (actions) | `crates/horizon-xr` |
| The eyes, the HUD panel, head aim, the controllers as a gamepad | `crates/horizon-client/src/gfx/xr.rs`, `game/pad.rs` (`xr_look`, `poll_pads`) |
| The laser pointer in menus | `gfx/xr.rs` (`xr_pointer`, `point`, `hit`, `laser`), `game/xr_pointer.rs` |
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
5. The HUD and menus go onto a panel (an OpenXR quad layer) 1.4 m wide, 1.5 m ahead and a little below the
   eyes; it is drawn again only when the HUD changed. While playing it follows the head once it turns 40 degrees
   away. When a menu, a screen or the title opens it is put in front of the head, and it stays there until it
   closes.
6. In a menu, the pointing hand's laser is a second quad layer: a thin strip from the controller's aim pose
   to where its ray meets the panel (1 m long when it misses), turned about the ray to face the head.

The head aims (`xr_look` turns the player by the head's yaw and sets its pitch). Controllers come in as one more
gamepad: sticks, triggers, grips, A/B/X/Y, stick clicks and Menu, bound for Touch, Index and the simple controller,
plus each hand's aim pose.

## Menus

As in Vivecraft, menus are used by pointing:

- Where the pointing hand's ray meets the panel, the game's pointer goes (window pixels, sent to the screens as
  the mouse's moves through `pad_pointer_to`), with the controller's arrow drawn there; the hovered button
  highlights.
- The trigger is the left mouse button (pressed past 0.6, released under 0.4), so clicks, drags and sliders
  work as with a mouse. A clicks too, B or Menu go back, the right stick scrolls (the pad's pointer mode).
- The right hand points; pulling the left trigger alone gives the pointer to the left hand, and the right
  trigger gives it back. A hand that isn't tracked hands it to the other.
- The left stick click puts the panel in front of the head again (not in inventories, where it throws a stack).

What Vivecraft does, from its public pages (the behaviour only, never its code): menus appear fixed just in front
of the player ([How to play](https://www.vivecraft.org/how-to-play/)), the main menu sits in a 3D "menu world"
([1.11 releases](https://github.com/jrbudda/Vivecraft_111/releases)), screens are used with the main hand's
pointer, the GUI scale is forced large so buttons are big enough to point at
([#267](https://github.com/Vivecraft/VivecraftMod/issues/267)), the left grip is shift in menus and the stick
scrolls ([controller setup](https://www.vivecraft.org/vivecraft-steamvr-input/),
[1.12 releases](https://github.com/jrbudda/Vivecraft_112/releases)), and a keyboard of its own opens on a text
field, typed on with the pointers ([FAQ](https://www.vivecraft.org/faq/)). Not here yet: a menu world, the larger
GUI scale, shift on the grip, a pointer keyboard (the pad's on-screen keyboard works through the laser), hover
haptics.

## Testing without a headset

```sh
cargo build --release -p horizon-client -p horizon-xr -p horizon-xr-fake
python3 tools/vr_fake_run.py --resources GAME_FOLDER_WITH_.local --out vr-run
python3 tools/vr_fake_run.py --resources ... --settings-from GAME_FOLDER --size 3132x3016 --hz 1000 \
    --world-frames 9000 --script tools/vr_fake_scripts/look_around.txt
python3 tools/vr_fake_run.py --resources ... --title --world-frames 1500 \
    --script tools/vr_fake_scripts/title_laser.txt --capture-seconds 5,8,11,14
```

The game runs from a temporary folder (the prepared resources linked read-only, a fresh world), on the fake
runtime through `XR_RUNTIME_JSON` for that process only. `vr-run/report.json` has the frame count, frame times
(all and the last 300 frames), the layers submitted and the actions created; `latest-*.png` are each eye and the
panel. `--title` stays on the title screen (`--frames` instead of a world). A script moves the controllers with
`T pose /user/hand/right/input/aim/pose YAW PITCH X Y Z` (untracked until then); `--capture-seconds` captures at
script times (`t5-*.png`), and every capture with a quad shown also writes `*-eyeN-composite.png`: each eye with
the panel and the laser drawn over it, as a headset shows them. `--hz 1000` never waits, to measure the game; `HORIZON_SPIKES=0` in the environment logs every frame's
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
| Distant terrain selects again at most every 50 ms as samples arrive, not every frame | 111 | 9.0 |
| Draws grouped by arena page in bands that widen with distance, translucent ones too past 256 blocks (3,700 calls an eye down to 425) | 174 | 5.8 |

What is left per frame (both eyes): building the draw lists (2.0 ms: 7,600 sections and 2,650 distant tiles,
sorted), the rest of each eye's preparation (sky, entities, clouds, uploads), visibility (0.6 ms), recording and
encoding the passes (0.3 ms); on the main thread, `update_world` (1.0 ms, most of it distant terrain).

## Not there yet

Room-scale movement, aiming with a controller in the world, the hand and held item, the hotbar on the off hand,
snap turning, a menu world, and an antialiasing that suits VR (MSAA with alpha to coverage). The menus' pointer
has only been tried on the fake runtime, never on a real headset.
