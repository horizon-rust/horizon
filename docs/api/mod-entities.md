# Experimental server entities

Stage E provides authoritative ephemeral records for server mods. Use `api = "0.3"`,
`side = "server"`, `capabilities = ["entities"]` and `experimental = ["entities"]`.
Import `horizon_mod_sdk::server::entities`. This candidate assigns the six genuinely new
functions provisional local level 9 on `1feb28c`, following provisional resource level 8.
Both collide with draft PR 2888. These are local test allocations, unsuitable for SDK,
registry or binary release. Reallocate on the merged substrate and rebuild guests before integration.

`register-types` atomically registers owned namespaced keys and returns opaque type handles.
Each definition supplies width, height, tracking range, a namespaced renderer key and public
contract version. The renderer contract is descriptive in E. A separate client program and
later drawing and transport gates are required to show these entities.

`context` returns the current host world-session identity and the dimensions actually loaded,
including parked levels. It returns none outside a world. It does not load dimensions or chunks.
`spawn` uses a registered type handle and state containing a dimension, feet transform and public
bytes. `set-state` atomically replaces that state. `despawn` removes the record. `poll` drains
lifecycle events. Direct state updates are kinematic and provide no collision, damage, AI, inventory,
save format, network packets or drawing. Private gameplay data must stay out of public bytes.

## Bounds and failures

- Eight types per running owner, 64 live entities per owner across a world and 256 per world.
- All entity functions share 64 attempted calls per callback, including rejected calls. Init gets
  the same allowance. Each state has at most 1024 public bytes. Poll retains the newest 256 events.
- Width and height must be finite and greater than zero, at most 64 blocks. Tracking range must
  be finite and greater than zero, at most 256 blocks. Public contract versions are 1 through 65535.
- Keys are at most 128 bytes and use namespaced lowercase resource syntax without empty or dot
  path segments. Type keys must belong to the owner's manifest namespaces. Renderer keys name
  the later client contract and may use another namespace.
- Positions and rotations must be finite. X and Z are within 29,999,984 blocks, yaw within
  360 degrees and pitch within 90 degrees. Feet Y must fit the loaded dimension height and the
  feet chunk must be resident. No guest request creates a ticket or loads a chunk.
- Handles identify their kind, owner, epoch, world, slot and generation. They are opaque strings.
  Foreign, forged, wrong-kind and stale handles fail. Never edit, transfer or persist them.
  IDs come from the game's shared allocator with checked conversion to positive network IDs.
  Exhaustion fails closed. Failed callback reservations consume no engine IDs or live slots.

## Transactions and lifetime

Registrations, spawns, updates, removals and poll consumption commit only after a successful
callback. A trap or command error discards all staged work. A trap also stops that owner and
synchronously revokes its existing records. Init errors discard registration and spawn reservations.
Reload first destroys the old owner, including when the replacement fails. Disabled owners are
revoked individually and healthy owners survive. No cleanup needs a final guest callback.

Each loaded level owns its records. Parking or swapping a level preserves them. Chunk unload,
dimension close and world close remove records immediately and report the corresponding poll
reason to a surviving owner. World reopening creates a fresh world identity. Empty level tables
are released. With no enabled entity program, no entity tables or queues are allocated.

## Verification

`cargo test --locked -p horizon-mods --features runtime,pack,index` compiles the original Rust
server guest and componentizes it. Tests package fresh nonce bytes after compilation and check
applied host records for folder and `.hmod` loads, rollback, quotas, invalid input, owner failure,
reload and world cleanup. They also compile an unchanged SDK from the immutable pre-entity base.
`TestGame` scripts can assert `entities`, `entity-state` and `entity-position` after guest callbacks.

`cargo test --locked -p horizon-client --features mods --test mod_entities` exercises the game's
per-level adapter using the same real guest. It can consume `HORIZON_ENTITY_TEST_COMPONENT` from
the sandbox build, otherwise it builds the native packer and guest in a separate target.

`tools/mod_entities_e2e.py` drives a rebuilt native game in isolated scratch homes through
`horizon-ctl chat "/entityhost ..."`, reads fresh events and checks opt-in host record traces.
`HORIZON_MOD_ENTITIES_TRACE` is a fixture-only JSON-lines destination. It records applied host
state after callbacks and cleanup, and grants no guest file access. This establishes Stage E
host records only. Visible and networked gameplay remain separate acceptance gates.

## Provisional visible meshes

The pinned private candidate adds `horizon_mod_sdk::models`, capability and experimental
opt-in `models`, on local level 10. This is not a released API allocation.
A separate client program registers `register_renderer(key, public_version)` at init.
The key must belong to its manifest namespaces. `views()` returns current authoritative
public records matching registered keys and versions. Each `View` has an opaque `handle`,
`kind`, `renderer`, `public_version: u32`, `dimension`, feet `x/y/z: f64`,
`yaw/pitch: f32`, `public_state: Vec<u8>` and `revision: u64`. State stays capped at 1024 bytes.

`upload_mesh(&[MeshVertex], &[Triangle], &png_bytes)` returns an owned opaque mesh handle.
`MeshVertex` fields `x/y/z/nx/ny/nz/u/v` are `f32`. `Triangle { a, b, c }` uses `u32`
indices. Positions are finite local feet coordinates within 64 blocks, normals within
-1 through 1, UVs within 0 through 1. The host applies entity yaw and pitch.
The host accepts at most 65536 vertices and 32768 triangles per mesh, 64 meshes per
program and 64 texture leases across the client. PNGs are decoded with a 128 MiB allocation
limit, at most 16 MiB encoded and 4096 by 4096 pixels. Larger textures are downsampled
with nearest sampling to fit 512 by 512 cells while preserving aspect ratio. Read owned material PNG bytes through
`resources::read`; no host path is accepted. The shared atlas reserves 64 texture cells.

`bind(view_handle, mesh_handle)` retains a draw binding. `update_mesh(mesh_handle, vertices)`
replaces positions, normals and UVs while preserving vertex count, indices and texture.
`release_mesh(mesh_handle)` removes the mesh and its bindings. All six calls share 256
attempts and 262144 uploaded vertices per callback. Traps and command errors roll back
mesh mutations. Owner stop releases its meshes. World change revokes mesh and view
handles. The host expands triangles to `[a,b,c,c]`, uses `mesher::Vertex` and the existing
entity upload, quad indices, lightmap, depth and alpha-cutout pipeline.

In-frame clients read the level's authoritative records. Integrated memory connections
and dedicated TCP clients receive the same bounded `horizon:entity_views_v1` custom
payload codec through the existing play connection and LAN hub. Custom records never
become vanilla spawn packets or vanilla type IDs. The hub filters each viewer by dimension,
sent chunks and tracking distance, sends removals before changed records, retains pending
updates by revision and limits extension output to 64 KiB per viewer per tick. One payload
is capped at 16 KiB. Only Horizon-branded clients that advertise a matching renderer key and public version
receive these views. Renderer advertisements use the existing serverbound CustomPayload
route and never create authoritative records. This private protocol still needs release
allocation and public compatibility policy.
`HORIZON_MOD_MODELS_TRACE` optionally records actual frame draw counts for original fixtures.

## Provisional world snapshots

Local level 11 adds `server::world_storage` with capability and experimental opt-in
`world-storage`. `context()` returns `Result<Option<u64>, String>` with an authoritative
world session token. Init has no world. `get(key)` returns `Result<Option<Vec<u8>>, String>`.
`set(key, bytes)` writes one complete owned snapshot through a temporary file, fsync and
atomic rename, then syncs the directory before acknowledging success. Keys use the existing
storage key syntax. Files live under the host-selected world root in `horizon-programs/<id>`.
No root path crosses the SDK. Eight MiB per program and eight MiB written per callback are
allowed, with at most 64 keys. `save-off` rejects writes. Use one versioned snapshot for
related gameplay records. This provides no atomic transaction with vanilla world files or
across several snapshot keys. Legacy settings storage remains separate.

The pinned private package inventory limit is 1 GiB, retaining 65536 entries and 16 MiB per file.

## Optional block collision movement

`horizon_mod_sdk::server::entities::move_colliding(entity: &str, delta: &MovementDelta)`
returns `Result<MovementResult, String>`. `MovementDelta` has `x`, `y`, `z` as `f64`.
`MovementResult` has the resulting feet `x`, `y`, `z` as `f64` and `on_ground: bool`.
It uses the existing `entities` capability and experimental opt-in. The new import has
provisional local level 14. This is not a released API allocation.

The host uses the owned type's width and height, resident server chunks, native block
collision scanning and native AABB clipping. A successful call stages only position and
revision, preserving dimension, rotation and public bytes. It follows the existing callback
commit and rollback rules. Direct `set_state` remains unconstrained by collisions.

Delta must be finite and at most four blocks per axis. Eight collision attempts are allowed
per callback within the shared 64 entity attempts. Each of at most three scans visits at
most 4096 candidate cells including margins. No chunk loads or world writes occur. The
start body must be unobstructed. Ground support is a separate downward 0.00001-block probe
at the resulting position and does not change position.

An unknown or stale handle, absent world, missing swept or support chunk, height boundary,
unsupported block context or exhausted bound returns `Err` without staging movement.
Contexts that do not lend native collision data return an explicit unsupported error.
Static cached block shapes are supported, including declared content template aliases.
Liquids, powder snow, scaffolding and states requiring dynamic or block entity shapes
currently return unsupported errors. Template physics retain their declared limitations.
This helper adds no stepping, gravity, velocity, entity collisions, world-border collision,
autonomous behavior or network authority. Gameplay chooses whether to handle an error
or explicitly use direct kinematic movement. Neither fallback is implicit in the host.

This source addition has not been compiled or exercised in a game.

## Empty-hand custom entity use

An empty main hand can right-click the nearest owned custom entity through the existing
optional server `item_used` callback. It receives `item_id = "minecraft:air"`, `count = 0`
and the authoritative owner-filtered `target`. The returned `consume` must be zero.
No SDK export, registration shape or renderer public JSON field changes.

The Horizon client sends one ordinary `UseItem` for an empty-main-hand custom click only
when a current advertised custom view has valid authoritative body dimensions and wins
the native entity and block ray. No entity ID or handle is sent by this route. The server
recomputes the ray from the authenticated actor's eyes and packet rotation, using registered
type dimensions and the current world records. Survival reach is three blocks, creative
reach five. Nearer native entities and block outlines, including equal-distance ties,
prevent selecting the custom target. Bounds are feet-centered X/Z with the registered
width and height, axis-aligned independently of mesh animation and yaw.

In-frame play dispatches this same empty-main-hand custom click once. It does not try a
block behind the custom target or a second offhand callback if the program passes.
Without an eligible custom target, the ordinary empty-hand input path is unchanged.
Existing sequence acknowledgements, inventory correction and bounded fallback suppression
remain in the normal `UseItem` server handler. Guest actions retain the nearest authenticated
other-player hit distance while the hub is lent to native code, restoring the prior value
for nested actions. With the hub present, native player boxes are queried live.

The private view transport optionally appends two bounded floats for registered width and
height. Both must be finite, positive and at most 64 blocks. Older messages without these
fields still draw but cannot trigger the client custom hit route. New bounded messages
require the matching pinned native decoder. This metadata is separate from SDK `View`
and renderer public JSON. It grants no client-side action or target authority.

This source path has not been compiled or exercised in a game.
