# Sound physics

An extra (off by default, [extras.md](extras.md)): **Options → Horizon Settings... → Extras...
→ Sound**. It does what Sound Physics Remastered (SonicEther's Sound Physics, continued by Max
Henkel, vlad2305m and Saint) does on Fabric and NeoForge:

- **Reverb by place.** Rays traced around a sound feed four reverb buses, from a small room
  (0.15 s) to a large cave (4.1 s). Stone rings, wool and moss deaden.
- **Occlusion by material.** The blocks between the sound and you muffle it: high frequencies
  first, then the level. Wool most, glass, snow and leaves-like blocks little, vines, ladders,
  chains and the like not at all. Half blocks count a quarter.
- **Underwater.** You or the sound in water: the sound is filtered, its reverb too.
- **Sound direction.** A sound behind a wall is heard from where its reflections reach you, such
  as an open door, at its real distance.

The mod cannot run in Horizon: it is a Fabric jar of Java mixins into Minecraft's `SoundEngine`
and OpenAL's EFX extension, and Horizon runs neither Java nor OpenAL (its mods are WebAssembly,
[mods.md](mods.md); its mixer is its own, `audio.rs`). So the mod's calculation is ported to
Rust and the EFX parts it drives are rebuilt in the mixer.

> **Licence.** Sound Physics Remastered is GPL-3.0 (its repository, henkelmax/sound-physics-remastered,
> and the jar's `fabric.mod.json`). `sound_physics.rs` follows its `SoundPhysics.java` and
> `ReflectedAudio.java` closely, and its block tables and reverb presets are the mod's. That
> makes these files a derivative of GPL-3.0 code inside an MIT project: they are for personal
> use on this branch and must not be distributed (no push, no merge into the public tree, no
> release) unless the licensing is settled first (the authors' permission, or a clean-room
> rewrite from a written description).

## Files

| Part | Code |
|---|---|
| Settings, materials, rays, the calculation | `crates/horizon-client/src/sound_physics.rs` (unit tested) |
| The thread that traces, the section snapshot | `crates/horizon-client/src/acoustics.rs` (unit tested) |
| EFX low-pass, the four reverb buses | `crates/horizon-client/src/reverb.rs` (unit tested) |
| Mixer: per-channel filters and sends, hold, redirect | `crates/horizon-client/src/audio.rs` (`mix`, `MixerBackend::starting`) |
| The hook where a channel starts | `sound_engine.rs`: `Backend::starting`, a no-op for every other backend |
| Game glue, once a tick | `crates/horizon-client/src/game/sound_physics.rs` |
| Options | `extras.rs` (`Item::Sound*`), stored as `soundPhysics.*` in `horizon-extras.txt` |
| Cost | `crates/horizon-client/examples/sound_physics_bench.rs` |

## How it runs

1. **A sound starts** (`SoundEngine.play` → `Backend::starting`, where the mod injects into
   `Channel.play`). A positional, attenuated sound is held in the mixer (40 ms at most) and a
   message goes to the `sound physics` thread. Relative and non-attenuated sounds (UI, music)
   are left alone, as are rain, thunder, goat horns (the mod's sound rate config turns them
   off), ambient loops unless `evaluate_ambient_sounds`, and music discs unless
   `update_moving_sounds`.
2. **The thread traces** `sound_physics::evaluate` over its snapshot:
   - occlusion: from the sound to the listener, block after block (`max_occlusion_rays`),
     adding each block's occlusion factor (× `non_full_block_occlusion_factor` on a face that
     is not sturdy); the least of nine paths shifted by `occlusion_variation` (one path with
     `strict_occlusion`, at least 0.49 for block sounds). Direct high-frequency gain
     `exp(-occlusion × block_absorption × 3)`, gain its 0.1th power.
   - reverb: `environment_evaluation_ray_count` rays on a golden spiral, each bounced
     `environment_evaluation_ray_bounces` times (mirror reflection, 256 blocks a leg). Each
     bounce adds to the bus its path length × the surface's reflectivity points to; the long
     buses are weighted by the mean reflectivity of their bounce raised to 3 and 4.
   - direction: every hit that sees the listener counts as shared airspace (which also opens
     the filters); with occlusion, the sound moves to the inverse-square-weighted mean of
     those directions.
   - underwater: direct high-frequency gain × (1 − `underwater_filter`), sends × 0.4.
3. **The mixer applies** the result: the direct path and four sends through an EFX-style
   low-pass (gain, gain above 5 kHz) with OpenAL's air absorption (0.99426 per block ×
   `air_absorption`), the sends into the four buses with OpenAL's room rolloff, the
   attenuation range divided by `attenuation_factor`. The buses are idle (and cost nothing)
   while nothing is sent to them and their tail has died.
4. **Every tick** the game gives the thread the sections of the 9 × 9 chunks around the
   listener that changed (`Arc<Section>` pointers, no block copied) and whether the camera is
   underwater. Opening a door reaches the next sound within a tick.

Turned off, nothing runs: no thread work, no sync, and the mixer is bit-for-bit vanilla
(`sound_physics_off_or_dry_leaves_the_mix_as_vanilla`).

## Settings

The Extras page has the switch, one switch per effect (Horizon's: the mod has only `enabled`),
Reverb gain, Block absorption, Rays (8, 16, 32, 64) and Ray bounces (2, 4, 8, 16). The file
takes all of the mod's general settings under their own names and ranges:

| Key (`soundPhysics.` …) | Default | Mod's |
|---|---|---|
| `enabled` | false | true |
| `reverb`, `occlusion`, `underwater`, `reflections` | true | (Horizon) |
| `attenuation_factor` | 1.0 | same |
| `reverb_attenuation_distance` | 0.0 | same |
| `reverb_gain`, `reverb_brightness` | 1.0 | same |
| `reverb_distance` | 1.5 | same |
| `block_absorption` | 1.0 | same |
| `occlusion_variation` | 0.35 | same |
| `default_block_reflectivity` | 0.5 | same |
| `default_block_occlusion_factor` | 1.0 | same |
| `air_absorption` | 1.0 | same |
| `underwater_filter` | 0.9 | same |
| `evaluate_ambient_sounds` | false | same |
| `environment_evaluation_ray_count` | 32 | same |
| `environment_evaluation_ray_bounces` | 4 | same |
| `non_full_block_occlusion_factor` | 0.25 | same |
| `max_occlusion_rays` | 16 | same |
| `max_occlusion` | 64 | same |
| `strict_occlusion` | false | same |
| `redirect_non_occluded_sounds` | true | same |
| `update_moving_sounds` | false | same |
| `sound_update_interval` | 5 | same |
| `max_sound_processing_distance` | 512 | same |

Not carried over: the per-block reflectivity and occlusion files (the mod's defaults are built
in, keyed by sound type), the per-sound rate file (its defaults are), `sound_distance_allowance`
(a server-side play range), the level clone settings (not needed, see step 4), the debug
rendering and logging, and Simple Voice Chat.

## Cost

Measured with `cargo run --profile fast -p horizon-client --example sound_physics_bench`
(Ryzen 7 7800X3D, Windows, `fast` profile, while other builds shared the machine: medians hold, the tails vary). The VR budget this matters for: about 7 ms of CPU a frame on the game thread
([vr.md](vr.md)).

**Game thread** (what counts against the VR frame):

| Work | When | Cost |
|---|---|---|
| Starting a sound | each sound | one message to the thread; no ray on the game thread |
| Section sync, 9 x 9 chunks x 24 sections | each tick (20 a second) | 0.008 ms unchanged, 0.016 ms with an edit in every chunk |
| First sync after turning it on or a teleport | once | 0.12 ms |
| Handing over the block states (30 000) | once, when turned on | 0.4 ms |
| Off | | one settings comparison a tick |

**Sound physics thread**, one sound, ms (median / 95th percentile of 200):

| Scene | 8 rays x 4 | 16 x 4 | 32 x 4 (default) | 32 x 8 | 64 x 4 |
|---|---|---|---|---|---|
| Open ground | 0.035 / 0.040 | 0.072 / 0.098 | 0.144 / 0.190 | 0.147 / 0.179 | 0.330 / 0.352 |
| Stone room with a door | 0.023 / 0.025 | 0.041 / 0.044 | 0.074 / 0.077 | 0.107 / 0.136 | 0.125 / 0.157 |
| Behind 3 blocks of wool | 0.037 / 0.040 | 0.074 / 0.080 | 0.149 / 0.159 | 0.147 / 0.163 | 0.298 / 0.356 |
| Large cave | 0.047 / 0.064 | 0.091 / 0.117 | 0.242 / 0.251 | 0.330 / 0.445 | 0.361 / 0.449 |

At the default, a busy scene of 100 sounds a second keeps that thread about 2 % busy; a sound
waits well under a millisecond before it starts (the hold is 40 ms at most). The materials are
built there too (2.7 ms, once).

**Audio thread**, mixing one second of 48 kHz stereo, ms:

| Voices | Vanilla mixer | With sound physics |
|---|---|---|
| 8 | 1.2 | 8.2 |
| 32 | 5.8 | 17.4 |
| 64 | 9.0 | 31.1 |

Most of the extra is the four reverb buses (about 7 ms a second while any rings); they stop
when silent. That is under 1 % of a core for a usual scene, on the audio thread, not the game
thread.

Run it again with `cargo run --profile fast -p horizon-client --example sound_physics_bench`.
In game, `horizon-ctl status` (the audio census) gives `soundPhysics`: sounds evaluated, mean
and max time on the thread, sounds played dry because it was behind, and the sync's mean and
max on the game thread.

## Differences from the mod

- Blocks are whole cubes for the rays: the mod clips each block's collision and fluid shapes,
  so a ray passes over a slab or between fence posts there. Faces still count as non-full for
  occlusion from the exported `isFaceSturdy` bits.
- The sound type of a block is read from its break sound, then its step sound
  (`block.<type>.break`); a sound type that borrows another's sounds gets that one's values.
- A ray that escapes always stops bouncing. The mod keeps bouncing it from its last hit, adding
  the distance to the listener again each time, while the path is shorter than
  `reverb_attenuation_distance` (off by default, so only with that setting).
- The reverb buses are a feedback delay network with the presets' decay, high-frequency decay,
  reflections and delays, not OpenAL Soft's EAX reverb: the decay times are measured in tests
  (`presets_decay_as_the_mod_sets_them`), the tone is not the same.
- The rays read the sections of 9 × 9 chunks around the listener, all heights (the mod clones
  the same range every 20 ticks); past them, rays escape.
- Entity sounds are not lifted to the entity's eyes, and the player's own sounds stay as
  vanilla plays them (the mod moves both).
- A burst of more than 64 sounds waiting for the thread plays the rest dry, where the mod
  traces every one on its sound thread.

## Not verified

Not listened to: everything above is checked by unit tests and the benchmark, not by ear.
The reverb's loudness against the mod's, the hold before a sound starts, and how it sounds in
VR (KRVR) are untested in a real game.
