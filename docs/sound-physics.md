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

Beside it, five extras of Horizon's own, each with its switch under the same header and
independent of sound physics (`crates/horizon-client/src/spatial.rs`):

- **Binaural sound** (`soundPhysics.hrtf`, off): 3D sound for headphones. Vanilla turns OpenAL's
  HRTF off; Horizon has a structural head model instead of measured responses (Brown and Duda,
  1998): each ear gets a sphere's interaural delay (up to 0.66 ms), its head shadow (a high
  shelf from +6 dB facing the sound to -20 dB in the deepest shadow) and the pinna's five
  echoes, whose delays carry elevation and front from back. No data files. It replaces the
  stereo panning of positional sounds; UI and music keep vanilla's.
- **Doppler effect** (`soundPhysics.doppler`, off; `doppler_factor` 1): OpenAL's formula, speed
  of sound 343.3 blocks a second, from the velocities of the sound and the listener measured
  off the positions the game sets (vanilla sets none). A jump faster than 100 blocks a second
  (a teleport) is no velocity, and one older than 150 ms lapses.

- **Arrow whoosh** (`soundPhysics.arrow_whoosh`, off): arrows in flight whistle, louder and
  higher the faster they go (silent under 8 blocks a second). Vanilla has no such sound; it is
  noise through a band-pass at 900 Hz with a slight flutter, synthesised once and looped, on the
  players' volume. Each arrow's whoosh goes through sound physics, binaural sound and Doppler
  like any sound.

- **Bow creak** (`soundPhysics.bow_creak`, off): drawing a bow creaks, harder and faster as the
  tension rises over the 20-tick draw (stick-slip jolts ringing two wood resonances, 220 Hz and
  600 Hz), synthesised once (1.2 s). It plays at the bow hand, right
  of the eyes, and fades out over a buffer on an early release. Vanilla's draw is silent.

- **Soft item pickup** (`soundPhysics.soft_pickup`, off): the item pickup pops lower and
  softer. Vanilla draws its pitch `((r1 - r2) * 0.7 + 1) * 2`, clamped by the engine to 0.5..2,
  so half the pops sit at 2, an octave up; with this on it is centred on 1.3 (a fifth lower)
  with half the spread (0.8 to 1.8), at 0.8 of the volume (`sound_physics::soft_pickup_pitch`).
- **Sound range** (`soundPhysics.sound_range`: Vanilla 1, 1.5 or 2 on the page, up to 3 in the
  file): how far sounds carry against vanilla's 16 blocks a unit of volume. It stretches the
  mixer's linear range (with the mod's `attenuation_factor`), the air paths' reach (± 16 × 12 ×
  16 blocks times the range, so directions through openings hold to the end of it) and the
  reverb's fade. In single player, alone on the integrated server, the server sends sounds that
  much further too (`Hub::sound`, through `sound_physics::shared_range`: the server has no
  settings of its own); with anyone else connected, or on any other server, only the player's
  own mix changes, so no one hears what the server does not send them. Cost: doubling the range
  covers four times the area, so in a busy place (a village, a farm) about four times the
  voices on the audio thread (see Cost), and a cold air flood of about 2 ms instead of 0.6 on
  the sound physics thread. Nothing on the game thread.

With any of them on, a voice's gains, filters and ears ramp over each 10 ms buffer, so a new
environment or a turn of the head does not click. All off, the mix is vanilla's to the bit.

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
| Binaural ears, Doppler, measured velocities | `crates/horizon-client/src/spatial.rs` (unit tested) |
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
     `environment_evaluation_ray_bounces` times (mirror reflection, 256 blocks a leg). The legs
     measure the room: their mean free path (4 V / S) and the mean energy each bounce keeps
     (`reflection`: the mod's reflectivity × 0.57, stone 0.86, wood 0.23, wool 0.06; a leg that
     escapes keeps nothing). Eyring's formula gives the decay time, `0.161 V / (-S ln(1 - a))`;
     it picks the two buses whose decays frame it (equal power on a log scale), at a level of
     the energy that comes back.
   - direction: every hit whose air cell the listener sees counts as shared airspace (which
     opens the filters, as in the mod). A sound the listener does not see is heard from the
     first point the listener sees on its air path: a flood through the air from the
     listener's cell (± 16 × 12 × 16 blocks, a sound's usual range) that goes round blocks,
     through open doors and not closed ones (`closes_axes` of the block's shape). Where no air
     path joins them (a closed room), the mod's average of the reflections' directions.
   - surfaces darken the reverb: each bounce keeps a share of the highs equal to its share of
     the energy (`reflection`: stone 0.86, wood 0.23, wool 0.06), and bus k, which holds later
     bounces, takes it k + 1 times (on stone the sends' high-frequency gain goes from 0.86 on
     the short bus to 0.55 on the long one).
   - near field: within `NEAR_FIELD` (2.5 blocks) of the listener the sends fall linearly to
     `NEAR_FLOOR` (0.3) at the ears, so the player's own sounds (an item picked up plays at
     the eyes) keep their place over the room.
   - underwater: direct high-frequency gain × (1 − `underwater_filter`), sends × 0.4, applied
     last.
   - a cache (`sound_physics::Cache`) keeps which cells the listener sees and the air paths
     while the listener stays in one cell and no block changes (light updates do not count).
   - moving on: when the listener enters another block, the sounds still playing for more
     than 0.4 s (loops, music discs with `update_moving_sounds`, long sounds) are traced
     again, 24 a tick at most (`update_on_listener_move`, Horizon's, on). The mod traces a
     sound once, when it starts.
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
Reverb gain, Block absorption, Rays (8, 16, 32, 64), Ray bounces (2, 4, 8, 16), then Binaural
sound and Doppler effect. The file
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
| `update_on_listener_move` | true | (Horizon) |
| `hrtf`, `doppler` | false | (Horizon) |
| `doppler_factor` | 1.0 | (Horizon; OpenAL's `AL_DOPPLER_FACTOR`) |
| `arrow_whoosh`, `bow_creak` | false | (Horizon) |
| `sound_range` | 1.0 | (Horizon; 1 to 3) |
| `soft_pickup` | false | (Horizon) |

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
| Section sync, 9 x 9 chunks x 24 sections | each tick (20 a second) | 0.008 ms unchanged, 0.017 ms with an edit in every chunk |
| First sync after turning it on or a teleport | once | 0.12 ms |
| Handing over the block states (30 000) | once, when turned on | 0.4 to 0.7 ms |
| Off | | one settings comparison a tick |

**Sound physics thread**, one sound, ms (median of 200: with the thread's cache, as repeated
sounds find it / with a cold cache, right after the listener moved a block or a block changed):

| Scene | 8 rays x 4 | 16 x 4 | 32 x 4 (default) | 32 x 8 | 64 x 4 |
|---|---|---|---|---|---|
| Open ground | 0.035 / 0.035 | 0.071 / 0.072 | 0.142 / 0.143 | 0.142 / 0.144 | 0.282 / 0.285 |
| Stone room with a door | 0.014 / 0.685 | 0.024 / 0.708 | 0.043 / 0.765 | 0.074 / 0.784 | 0.085 / 0.784 |
| Behind 3 blocks of wool | 0.037 / 0.467 | 0.075 / 0.507 | 0.148 / 0.591 | 0.148 / 0.585 | 0.297 / 0.748 |
| Large cave | 0.035 / 0.055 | 0.056 / 0.091 | 0.108 / 0.173 | 0.193 / 0.306 | 0.214 / 0.340 |

The cold cost of a hidden sound is the air flood (about 0.6 ms); the mod's version of the
calculation (no cache, no flood) took 0.07 to 0.25 ms at the default. At 100 sounds a second
the thread stays under 10 % busy even with every sound cold, and a sound waits under a
millisecond before it starts (the hold is 40 ms at most). The materials are built there too
(3 to 5 ms, once).

**Audio thread**, mixing one second of 48 kHz stereo, ms:

| Voices | Vanilla mixer | With sound physics | With binaural sound | Both |
|---|---|---|---|---|
| 8 | 1.6 | 8.6 | 8.8 | 14.4 |
| 32 | 6.4 | 20.3 | 35.8 | 47.8 |
| 64 | 18.5 | 44.2 | 89.6 | 115.7 |

Most of sound physics' share is the four reverb buses (about 7 ms a second while any rings);
they stop when silent. Binaural sound costs about 1.3 ms a second per voice. A usual scene
(10 to 20 voices) with both stays around 2 to 4 % of a core, on the audio thread, not the game
thread. The Doppler effect costs nothing measurable.

Run it again with `cargo run --profile fast -p horizon-client --example sound_physics_bench`.
In game, `horizon-ctl status` (the audio census) gives `soundPhysics`: sounds evaluated, mean
and max time on the thread, sounds played dry because it was behind, and the sync's mean and
max on the game thread.

## Differences from the mod

What the port fixes in the mod's calculation (the faithful port is commit `a69589b2d` on this
branch, to compare by ear):

- **Small stone rooms rang like cathedrals.** The mod picks the bus from each path's length ×
  the surface's reflectivity, and weights the long buses by the bounces' mean reflectivity,
  so every closed stone room saturated all four buses. The decay time now comes from the
  room the rays measure (Eyring): a 5-block stone room rings under two seconds, a 40-block
  cave over three, a wool room briefly (`rooms_and_caves_ring_for_their_size_and_walls`).
- **Direction through doors depended on luck.** With 32 rays a 1 × 2 doorway was often missed
  and the direction changed from one sound to the next. The air path finds the doorway every
  time, at 8 rays as at 64, and tells an open door from a closed one
  (`sound_comes_through_the_door`).
- **Up to 160 visibility rays a sound.** The mod traces one to the listener from every hit;
  they are cached per cell between sounds (`the_cache_answers_repeated_sounds_the_same`).
- **The underwater filter was partly undone** by the shared-airspace floor applied after it
  (`max(0.2 √shared, cutoff)`); it applies last now.
- **Inconsistent weights:** the short bus was the only one not weighted by reflectivity (gone
  with the new reverb), and the direct path counted with weight 1 against 1 / length² for the
  reflections; it is weighted as they are.
- **Mining in a row in a 1 × 2 stone tunnel was piercing.** The faithful port sent each item
  pickup pop (played at the eyes, at pitch 2 half the time) 0.84, 1.0, 0.95 and 0.71 into the
  four buses, high frequencies untouched: the pop rang on for seconds, its ring only 6.6 dB
  under the pop, in tonal bands around 2.5, 4.5, 6, 8 and 10 kHz (the long buses' modes). With
  the room reverb, the surfaces' high-frequency absorption and the near field, the ring sits
  30 dB under the pop, 24 dB less of it in 2 to 8 kHz and 27 dB less above (measured offline on
  `random/pop.ogg` through the mixer).
- **A ray that escaped kept bouncing** from its last hit while the path was shorter than
  `reverb_attenuation_distance`, adding the distance to the listener each time; it stops.

Other differences:

- Blocks are whole cubes for the rays: the mod clips each block's collision and fluid shapes,
  so a ray passes over a slab or between fence posts there. Faces still count as non-full for
  occlusion from the exported `isFaceSturdy` bits, and the air path uses the block's shape.
- The sound type of a block is read from its break sound, then its step sound
  (`block.<type>.break`); a sound type that borrows another's sounds gets that one's values.
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
The faithful port (`a69589b2d`) and the fixed one (`e95b1364f`) were played in a real game (a
deepslate cave; 0.24 and 0.32 ms a sound on the thread, 0.02 and 0.03 ms a tick on the game
thread), and the fixed one listened to by the user. Binaural sound, the Doppler effect and
tracing again on the move have not been listened to yet. The
reverb's loudness against the mod's, the hold before a sound starts, and how it sounds in VR
(KRVR) are untested by ear.
