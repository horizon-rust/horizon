# VR (experimental)

Horizon draws to a headset through OpenXR, on any runtime (SteamVR, Monado, Meta, Virtual Desktop...). It is a
research spike: off unless the game starts with `HORIZON_XR=1`, Vulkan only, and the OpenXR loader
(`openxr_loader.dll` on Windows, from the Khronos OpenXR-SDK releases) beside the game. Without the loader or a
headset the game logs why and starts as usual.

| Part | Where |
|---|---|
| OpenXR session, Vulkan through the runtime, swapchains, frame loop, controllers (actions) | `crates/horizon-xr` |
| Eye tracking (the gaze each frame), the quad views | `crates/horizon-xr` (`Frame::gaze`, `Frame::insets`, `controls.rs`) |
| The eyes, the HUD panel, head aim, the controllers as a gamepad | `crates/horizon-client/src/gfx/xr.rs`, `game/pad.rs` (`xr_look`, `poll_pads`) |
| Aiming at blocks with the eyes | `gaze_aim.rs` (smoothing, hold), `game/xr_gaze.rs`, `game.rs` (`pick`) |
| The laser pointer in menus | `gfx/xr.rs` (`xr_pointer`, `point`, `hit`, `laser`), `game/xr_pointer.rs` |
| The title's backdrop around the player | `gfx/xr.rs` (`backdrop`), `gfx/panorama.rs` (`Backdrop`), `gfx/panorama.wgsl` (`fs_backdrop`) |
| A fake runtime for tests without a headset | `crates/horizon-xr-fake`, `tools/vr_fake_run.py` |
| wgpu-core reading the environment's flags for an instance made from wgpu-hal | `third_party/wgpu-core` (`src/instance.rs`) |
| Variable rate shading for the eyes (a shading rate attachment, pipeline twins) | `crates/horizon-xr/src/foveation.rs`, `third_party/wgpu-hal` (`src/vulkan/`, marked "Horizon") |

## How a frame is drawn

1. `xrWaitFrame` paces the game; the head and both eyes are located in the stage space.
2. Each eye draws the world through the renderer's offscreen path into its swapchain image, at the headset's size,
   with its off-axis projection. The body's yaw is the player's less the head's, and the head's pose goes on top.
   The sky folds each eye's off-centre frustum in, so both eyes see the same sky. On the title the eyes draw
   its backdrop instead (below).
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

On the title (and the screens opened from it) the eyes don't show the window's spinning panorama, which would be
the same flat picture in both eyes, carried along by the head. They see the panorama as a scene around the
player, fixed in the stage: it is painted onto a floor under where the head was on the first frame and a sphere
30 m around that point (a ground-projected sky box). Each eye's pixel casts a ray from that eye, with its own
pose and field of view, and samples the cube towards the point it meets as seen from the centre, so both eyes
see a point of the scene in the same texel from 64 mm apart: the floor in front has the parallax of something
a few metres away, the sky and hills that of something 30 m away, and nothing moves when the head turns. The
floor is the stage's (the head's height on the first frame) when the reference space starts at the floor, else
1.62 m (vanilla's eye height) below the head. The panorama doesn't spin in VR. What stands up in the picture
(trees, hills) has the depth of the floor or the sphere behind it, not its own: a real menu world would.

What Vivecraft does, from its public pages (the behaviour only, never its code): menus appear fixed just in front
of the player ([How to play](https://www.vivecraft.org/how-to-play/)), the main menu sits in a 3D "menu world"
([1.11 releases](https://github.com/jrbudda/Vivecraft_111/releases)), screens are used with the main hand's
pointer, the GUI scale is forced large so buttons are big enough to point at
([#267](https://github.com/Vivecraft/VivecraftMod/issues/267)), the left grip is shift in menus and the stick
scrolls ([controller setup](https://www.vivecraft.org/vivecraft-steamvr-input/),
[1.12 releases](https://github.com/jrbudda/Vivecraft_112/releases)), and a keyboard of its own opens on a text
field, typed on with the pointers ([FAQ](https://www.vivecraft.org/faq/)). Not here yet: a menu world (the
backdrop above is the panorama given depth), the larger GUI scale, shift on the grip, a pointer keyboard (the pad's
on-screen keyboard works through the laser), hover haptics.

## The eyes

When the runtime has `XR_EXT_eye_gaze_interaction` and says the headset supports it
(`XrSystemEyeGazeInteractionPropertiesEXT`), `horizon-xr` binds a gaze pose to
`/interaction_profiles/ext/eye_gaze_interaction` (`/user/eyes_ext/input/gaze_ext/pose`) and locates it every
frame in the head's space; the stage direction is the head's pose turned onto it (`Frame::gaze`: both
directions, and whether the eyes are tracked or only held). A gaze that isn't valid is `None`, and so is the
gaze of a runtime without the extension: the head aims, as before, with nothing logged as an error.

While playing, the eyes aim at blocks: the outline, mining, placing and using items on a block follow the
block looked at, within reach, from the player's eye. The head still turns the player, and entities (mobs,
item frames, boats, armor stands) and projectiles still aim with the head. `HORIZON_XR_AIM=head` starts with
the head aiming at blocks too; clicking both sticks at once switches between eyes and head (the action bar
says which). Without eye tracking the switch says so and the head keeps aiming.

A coarse gaze is smoothed and held (`gaze_aim.rs`) so the outline never flickers: the gaze is smoothed over
about 40 ms, a block stays targeted while the gaze is within 7 degrees of where it was hit, and a new block (or
nothing, the sky) is taken once the gaze has rested on it for 0.12 s. A block that is broken gives way at once.

### The Vision Pro through KRVR (measured with a probe, KRVR 2.0.6.3)

KRVR XR Runtime 2.0.0 offers `XR_EXT_eye_gaze_interaction`, `XR_FB_eye_tracking_social`, `XR_VARJO_quad_views`
and `XR_VARJO_foveated_rendering`, at 4096x3288 per eye and 90 Hz. The gaze follows the eyes at once, but Apple
gives streaming apps an approximate one: over 10,775 samples it took 30 positions, never less than 10 degrees
apart, on a hexagonal grid about 12 degrees between neighbours (yaw from about -28 to +32 degrees, pitch from
-29 to +20). `XR_FB_eye_tracking_social`'s two eyes are the same (no vergence), and the runtime sometimes says
the gaze is valid but not tracked: Horizon still uses it. That is enough for blocks, which are big, within 5
blocks; it is not enough for small things, and Apple may have KRVR take it away, so everything works without it.

Through SteamVR 2.18.4 and KRVR's driver only `XR_EXT_eye_gaze_interaction` is there, without quad views,
and the gaze's pitch is upside down (a driver bug, reported): `HORIZON_XR_GAZE_INVERT_PITCH=1` mirrors it.
KRVR's own runtime needs no such fix. Its log says suggesting the eye gaze profile starts its foveated
streaming, which `horizon-xr` does whenever eye tracking is there.

## Foveated rendering (quad views)

`horizon-xr` can open the quad views (`Xr::new`'s `foveated`): when the runtime offers `XR_VARJO_quad_views`
(KRVR offers them only to an instance that enables the extension), the session is `PRIMARY_QUAD_VARJO`, each
eye wide at a low resolution plus an inset at a high one (`Frame::insets`, `Xr::inset_size`), submitted as four
views of one projection layer. With `XR_VARJO_foveated_rendering` supported, the inset sizes are the foveated
ones (`XrFoveatedViewConfigurationViewVARJO`) and `XrViewLocateFoveatedRenderingVARJO` is chained into
`xrLocateViews`, so the runtime puts the insets where the eyes look. Without the extensions the session is
stereo, as before.

The game draws them with `HORIZON_XR_FOVEATED=1` (off by default; it logs the inset size, or that the runtime
has no quad views and the session is stereo). `render_eyes` draws four views: the cull frustum stays the wide
eyes' (the insets are inside them), views 1 to 3 reuse view 0's visible sections and draw lists, each view has
its own projection and sky shear, the depth target is kept per size (`eye_depth`), and the window mirrors view
0. Shader packs are left out of the quad views for now: their targets (`pack_post`, `native_post`) are sized
for one view and would be made again four times a frame.

### Measured: stereo against the quad views

Fake runtime, RTX 4090, unpaced (`--hz 1000`), `tools/vr_fake_scripts/look_around.txt`, render distance 64,
distant terrain 1024, 9,000 frames in a fresh world; the log's timing line (`Horizon VR: 300 frames...`), the
median over its last nine lines. CPU is the game thread's frame without its waits (the headset's pacing, each
view's GPU waits, the window's image); GPU is the eyes' draws by timestamp queries, every view timed. The quad
insets are 1120x1120: 40 degrees across at the 4096-wide eye's sharpness in its centre, more than a 12-degree
gaze step on each side.

| Views | Pixels | CPU ms a frame | GPU ms a frame |
|---|---|---|---|
| Stereo, 4096x3288 an eye (the Vision Pro's through KRVR) | 26.9 M | 7.2 | 2.55 |
| Stereo, 2048x1644 an eye | 6.7 M | 6.9 | 2.3 |
| Quad: 2048x1644 wide, 1120x1120 insets | 9.2 M | 8.7 | 4.3 |

- **VR is limited by the CPU here, not the GPU**: the GPU works 2.5 ms of the 7 ms a frame takes.
- **Pixels hardly cost**: a quarter of the pixels saves 0.25 ms of GPU. A view's cost is mostly its own,
  whatever its size (the sections drawn, the sky, entities, clouds, the passes).
- **So the quad views are slower** on both sides: four views instead of two add 1.5 ms of CPU (recording and
  encoding the views) and 1.75 ms of GPU. They stay opt-in, and stereo stays the default.
- They could pay where pixels do cost: a weaker GPU, supersampling, or shader packs, which don't draw in the
  eyes yet (a pack draws through the world post's target, which the eyes skip). KRVR streams about 1648x1328 an
  eye to the headset, so most of 4096x3288 is downscaled anyway; but drawing fewer pixels saves little here
  either, for the same reason.

The inset must stay bigger than the gaze's step: the runtime chooses its field of view, so check in the headset
that a 12-degree jump never leaves the eyes on the wide view's edge; if it does, ask for fixed foveation
(`foveatedRenderingActive` false: centred, wider insets) or stay stereo.

## Foveated rendering without more views (variable rate shading)

`HORIZON_XR_VRS=1` (off by default, stereo only): each eye keeps its one view at the headset's size, and the
GPU shades it coarser away from where the eyes look (`VK_KHR_fragment_shading_rate`,
`crates/horizon-xr/src/foveation.rs`). A small `R8_UINT` image per eye, one texel for 16x16 pixels, says the
rate: every pixel within 25 degrees of the gaze (a 12-degree step on each side stays sharp), one fragment for
2x2 pixels to 40 degrees, one for 4x4 beyond. Without a gaze it is centred ahead (fixed foveation). It is
filled again only when the gaze moves.

wgpu has no shading rate, so the patched `third_party/wgpu-hal` does it: a render pass of the eye's size gets
the image as one more attachment (`vkCreateRenderPass2`, `Device::set_fragment_shading_rate`), and since such a
pass isn't compatible with pipelines made for the others, every render pipeline gets a twin taking the
attachment's rate, bound inside those passes (`RenderPipeline::raw_shading_rate`). `horizon-xr` turns the
extension and its features on in the device it creates, and makes the images with raw Vulkan (wgpu has no such
usage), filled through wgpu as textures.

Only shading gets cheaper: the geometry, the draws and the CPU stay the same, so it costs no CPU (unlike the
quad views). Ray queries made from a fragment shader (as the ray-tracing spike's shadows are) get cheaper with
it too, one ray per coarse fragment; a ray pass of its own (compute) isn't rasterized and has to read the rate
itself ([raytracing.md](raytracing.md)).

Measured like the table above, in a world kept from run to run (`--home`), 4,000 frames with the head still,
the last five timing lines: GPU 1.84-1.93 ms a frame with it against 1.88-2.08 without, about 0.1 ms (5 %),
CPU the same. Small, because the GPU waits for the CPU here and pixels are cheap; it grows with what each pixel
costs. A capture shows the 4x4 blocks at the eye's edge and the centre identical to stereo. Not checked against
the Vulkan validation layers (not installed here), nor in a headset.

## Testing without a headset

```sh
cargo build --release -p horizon-client -p horizon-xr -p horizon-xr-fake
python3 tools/vr_fake_run.py --resources GAME_FOLDER_WITH_.local --out vr-run
python3 tools/vr_fake_run.py --resources ... --settings-from GAME_FOLDER --size 3132x3016 --hz 1000 \
    --world-frames 9000 --script tools/vr_fake_scripts/look_around.txt
python3 tools/vr_fake_run.py --resources ... --title --world-frames 2200 \
    --script tools/vr_fake_scripts/title_laser.txt --capture-seconds 6,11,15,18
python3 tools/vr_fake_run.py --resources ... --title --world-frames 1300 \
    --script tools/vr_fake_scripts/title_backdrop.txt --capture-seconds 5,8,11 --out vr-run
python3 tools/vr_backdrop_check.py vr-run
```

The game runs from a temporary folder (the prepared resources linked read-only, a fresh world), on the fake
runtime through `XR_RUNTIME_JSON` for that process only. `vr-run/report.json` has the frame count, frame times
(all and the last 300 frames), the layers submitted and the actions created; `latest-*.png` are each eye and the
panel. `--title` stays on the title screen (`--frames` instead of a world). A script moves the controllers with
`T pose /user/hand/right/input/aim/pose YAW PITCH X Y Z` (untracked until then); `--capture-seconds` captures at
script times (`t6-*.png`), and every capture with a quad shown also writes `*-eyeN-composite.png`: each eye with
the panel and the laser drawn over it, as a headset shows them. `vr_backdrop_check.py` checks the title's
backdrop from the captures: the left eye turned 20 degrees, warped back through the fake headset's field of view,
matches the eye before the turn (it would not if the backdrop followed the head), and the floor in front shifts
more between the eyes than the sky does (1600x1600 an eye: 36 pixels against 2, and a warp error of 0.007
against 0.12 for a backdrop carried by the head). `--hz 1000` never waits, to measure the game; `HORIZON_SPIKES=0` in the environment logs every frame's
spans (`vr_eyes`, `vr_eye_draw`, `draw_lists`, `passes`, `encode`, `vr_panel`...). The fake runtime's settings
and script format are in `crates/horizon-xr-fake/src/lib.rs`.

The fake headset tracks the eyes unless `HORIZON_FAKE_XR_NO_EYES=1`; a script moves them relative to the head
with `T pose /user/eyes_ext/input/gaze_ext/pose YAW PITCH` (no valid gaze before its first step: the head
aims), and `HORIZON_FAKE_XR_GAZE_STEP=12` snaps them to the Vision Pro's hexagonal grid.
`tools/vr_fake_scripts/gaze_blocks.txt` aims at the ground with the eyes while the head stays still.
`HORIZON_FAKE_XR_QUAD=1` offers the quad views, the insets centred on the gaze. Without the game's resources,
the probe checks the OpenXR side alone:

```sh
cargo build -p horizon-xr -p horizon-xr-fake
# beside horizon-xr-probe: openxr_loader.dll, horizon_xr_fake.dll and its manifest (vr_fake_run.py writes one)
XR_RUNTIME_JSON=.../horizon-xr-fake.json HORIZON_FAKE_XR_SCRIPT=gaze.txt HORIZON_FAKE_XR_GAZE_STEP=12 \
    HORIZON_FAKE_XR_QUAD=1 horizon-xr-probe --foveated 7
```

It prints whether there is eye tracking, every distinct gaze direction it saw (in the head, degrees), and with
the quad views the insets' size and every place their centre went. On a real headset the same probe, without
`XR_RUNTIME_JSON`, shows what the runtime gives.

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

## HDR test pattern

`horizon-xr-probe` prints the runtime's extensions and swapchain formats, then clears the eyes. With `--hdr` it
shows a panel 1.5 m ahead (placed during its first two seconds, then fixed) of 12 grey patches labelled in nits,
10 to 4000, drawn into `R16G16B16A16_SFLOAT` swapchains as scRGB (linear, 1.0 = 80 nits); `--sdr` draws the
same values into the 8-bit sRGB swapchains, to compare. The brightest patch still brighter than the one before
it is as far as the runtime and headset go.

```sh
cargo build --release -p horizon-xr --bin horizon-xr-probe
cp .local/openxr-loader/openxr_loader.dll target/release/
target/release/horizon-xr-probe --hdr 300
```

KRVR 2.0.0 (Apple Vision Pro), 2026-10-27: it accepts `R16G16B16A16_SFLOAT` but has no colour space or HDR
extension (`XR_FB_color_space` and the like), and its stream is HEVC Main10 tagged full-range BT.709 / sRGB,
each value clamped to 0..1 before the 10-bit quantization (strings in `krvr_xr2.dll`, `wire: NVENC HEVC session
up` in `%LOCALAPPDATA%\KRVR\logs\runtime_*.log`): through it, everything past scRGB 1.0 is SDR white. In the
headset, 10, 20 and 40 nits got brighter and every patch from 80 or 100 up looked the same: no HDR through KRVR.

## Not there yet

Room-scale movement, aiming with a controller in the world, the hand and held item, the hotbar on the off hand,
snap turning, a menu world, and an antialiasing that suits VR (MSAA with alpha to coverage). The menus' pointer
has only been tried on the fake runtime, never on a real headset. Shader packs in the eyes (and so in the quad
views), and the eyes aiming at entities.

Not yet tried on the Vision Pro through KRVR, only on the fake runtime: the gaze's reading and binding, the
eyes aiming at blocks in the game (its smoothing and hold are unit tested, `gaze_aim.rs`; their angles and
times may want tuning in the headset), `HORIZON_XR_GAZE_INVERT_PITCH` through SteamVR, and the quad views:
whether KRVR offers them to the game, the inset sizes and field of view it gives, whether its insets follow the
gaze, whether the seam between inset and wide view shows, and what they cost on KRVR's own frame budget.
