//! The game's side of `NaturalSpawner` (`mobs::spawn_selection`): the level
//! it reads, the spawning chunks and caps of `ServerChunkCache.tickChunks`,
//! and the mobs it adds (finalized with the game's `finalizeSpawn`).
use super::{Dim, Game};
use horizon_client::vanillagen::VanillaTerrain;
use horizon_client::{
    biome::{BiomeInfo, SpawnEntry},
    blocks::BlockFacts,
    collisions::{SceneCollisions, no_collision_empty},
    mobs::{
        Mob, MobWorld, Species,
        data::{self, MobData},
        finalize::{self, Ctx, FinalizeLevel, Group, Reason, SpawnData},
        seen_grid::SeenGrid,
        spawn_rules::{Dimension, SpawnLevel},
        spawn_selection::{
            self, Category, Counted, Heightmap, NaturalLevel, SpawnCost, SpawnPlayer, SpawnRequest,
            SpawnState, Spawned, StructureAt, StructureSpawns,
        },
    },
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::{LegacyRandom, RandomSource, WorldgenRandom, XoroshiroRandom},
};
use horizon_world::{
    nbt::{Compound, Tag},
    registry::BlockStateId,
};
use std::collections::HashMap;

/// Natural spawning's candidate chunks (`ChunkMap.collectSpawningChunks`):
/// the union of every player's 17×17 chunk square, in player then dx/dz
/// order, each chunk once. `candidates` is one pass's working list (spawning
/// shuffles and drains it); the rest is the union for the players' last chunk
/// positions, kept between ticks. A chunk crossing rebuilds only the squares
/// it can change, and proximity to players is checked live on every pass.
#[derive(Default)]
pub(super) struct SpawnScratch {
    pub(super) candidates: Vec<(i32, i32)>,
    /// Each player's chunk at the last pass.
    chunks: Vec<(i32, i32)>,
    /// For each player, the others whose squares overlap its own, in order.
    neighbours: Vec<Vec<usize>>,
    /// The chunks each player adds to the union: those of its square, in
    /// dx/dz order, that no earlier player's square holds.
    segments: Vec<Vec<(i32, i32)>>,
    stale: Vec<bool>,
}
impl SpawnScratch {
    fn chunk(p: &LevelPlayer) -> (i32, i32) {
        (
            p.position.x.floor() as i32 >> 4,
            p.position.z.floor() as i32 >> 4,
        )
    }
    /// Whether the square around chunk `centre` holds chunk `c`.
    fn holds(centre: (i32, i32), c: (i32, i32)) -> bool {
        centre.0.abs_diff(c.0) <= 8 && centre.1.abs_diff(c.1) <= 8
    }
    fn overlap(a: (i32, i32), b: (i32, i32)) -> bool {
        a.0.abs_diff(b.0) <= 16 && a.1.abs_diff(b.1) <= 16
    }
    /// Bring the segments up to date with the players' chunks. A segment
    /// depends only on its player's chunk and those of the earlier players
    /// whose squares overlap it, so a move rebuilds the mover's segment and
    /// the later ones its old or new square overlaps.
    fn prepare(&mut self, players: &[LevelPlayer]) {
        let n = players.len();
        let all = n != self.chunks.len();
        self.stale.clear();
        self.stale.resize(n, all);
        if !all {
            let mut moved = false;
            for m in 0..n {
                let (old, new) = (self.chunks[m], Self::chunk(&players[m]));
                if old == new {
                    continue;
                }
                moved = true;
                self.stale[m] = true;
                for (i, p) in players.iter().enumerate().skip(m + 1) {
                    let c = Self::chunk(p);
                    self.stale[i] |= Self::overlap(old, c) || Self::overlap(new, c);
                }
            }
            if !moved {
                return;
            }
        }
        self.chunks.clear();
        self.chunks.extend(players.iter().map(Self::chunk));
        self.neighbours.resize_with(n, Vec::new);
        self.neighbours.truncate(n);
        self.segments.resize_with(n, Vec::new);
        self.segments.truncate(n);
        let chunks = &self.chunks;
        for (i, near) in self.neighbours.iter_mut().enumerate() {
            near.clear();
            near.extend((0..n).filter(|&j| j != i && Self::overlap(chunks[i], chunks[j])));
        }
        for (i, segment) in self.segments.iter_mut().enumerate() {
            if !self.stale[i] {
                continue;
            }
            let near = &self.neighbours[i];
            let earlier = &near[..near.partition_point(|&j| j < i)];
            let (pcx, pcz) = chunks[i];
            segment.clear();
            for dx in -8..=8 {
                for dz in -8..=8 {
                    let c = (pcx + dx, pcz + dz);
                    if !earlier.iter().any(|&j| Self::holds(chunks[j], c)) {
                        segment.push(c);
                    }
                }
            }
        }
    }
    /// Fill `candidates` with the union's chunks, in order, that are within
    /// 128 blocks of a non-spectator holding them in its square and that
    /// `keep` accepts. Returns the union's size.
    fn collect(
        &mut self,
        players: &[LevelPlayer],
        mut keep: impl FnMut((i32, i32)) -> bool,
    ) -> i32 {
        self.prepare(players);
        self.candidates.clear();
        let mut count = 0;
        for (i, segment) in self.segments.iter().enumerate() {
            count += segment.len();
            let near = &self.neighbours[i];
            for &c in segment {
                let close = close_enough_for_spawning(&players[i], c)
                    || near.iter().any(|&j| {
                        Self::holds(self.chunks[j], c) && close_enough_for_spawning(&players[j], c)
                    });
                if close && keep(c) {
                    self.candidates.push(c);
                }
            }
        }
        count as i32
    }
}

/// A player in the level in place, as spawning and despawning see it.
pub(super) struct LevelPlayer {
    pub(super) uuid: u128,
    pub(super) position: Vec3,
    pub(super) bounds: Aabb,
    pub(super) alive: bool,
    pub(super) creative: bool,
    pub(super) spectator: bool,
}
impl LevelPlayer {
    fn distance_sqr(&self, x: f64, y: f64, z: f64) -> f64 {
        (self.position.x - x).powi(2)
            + (self.position.y - y).powi(2)
            + (self.position.z - z).powi(2)
    }
    /// `Level.hasNearbyAlivePlayer`'s test of one player (any distance for a
    /// negative range).
    fn alive_near(&self, x: f64, y: f64, z: f64, range: f64) -> bool {
        self.alive && !self.spectator && (range < 0.0 || self.distance_sqr(x, y, z) < range * range)
    }
}
/// `ChunkMap.euclideanDistanceSquared` below 128 blocks
/// (`playerIsCloseEnoughForSpawning`).
fn close_enough_for_spawning(p: &LevelPlayer, (cx, cz): (i32, i32)) -> bool {
    let (x, z) = (
        f64::from(cx * 16 + 8) - p.position.x,
        f64::from(cz * 16 + 8) - p.position.z,
    );
    !p.spectator && x * x + z * z < 16384.0
}
/// `EntityGetter.getNearestPlayer(x, y, z, -1, false)`: the nearest
/// non-spectator, alive or not.
pub(super) fn nearest_player(
    players: &[LevelPlayer],
    x: f64,
    y: f64,
    z: f64,
) -> Option<&LevelPlayer> {
    let mut best: Option<(&LevelPlayer, f64)> = None;
    for p in players.iter().filter(|p| !p.spectator) {
        let d = p.distance_sqr(x, y, z);
        if best.is_none_or(|(_, b)| d < b) {
            best = Some((p, d));
        }
    }
    best.map(|(p, _)| p)
}

/// [`nearest_player`] for many positions: the non-spectators by the 64-block
/// square they stand in, searched ring by ring outwards from the position.
/// Every mob asks each tick, and with players spread over the map that was a
/// look at every player for every mob.
pub(super) struct NearestPlayers<'p> {
    players: &'p [LevelPlayer],
    cells: horizon_client::fast_hash::FxHashMap<(i64, i64), Vec<usize>>,
    /// The cells' bounding box: (x0, z0, x1, z1).
    bounds: (i64, i64, i64, i64),
    /// Few players, or one too far out for cells: every question scans.
    scan: bool,
}
impl<'p> NearestPlayers<'p> {
    const CELL: f64 = 64.0;
    /// Further out than any world border: such positions are answered by a scan.
    const FAR: f64 = 1.0e9;
    pub(super) fn new(players: &'p [LevelPlayer]) -> Self {
        let mut cells = horizon_client::fast_hash::FxHashMap::<(i64, i64), Vec<usize>>::default();
        let mut bounds = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
        let mut scan = players.len() <= 8;
        for (i, p) in players.iter().enumerate().filter(|(_, p)| !p.spectator) {
            if !(p.position.x.abs() < Self::FAR && p.position.z.abs() < Self::FAR) {
                // Past any world border (or not a number): scans answer.
                scan = true;
                break;
            }
            let key = Self::cell(p.position.x, p.position.z);
            cells.entry(key).or_default().push(i);
            bounds = (
                bounds.0.min(key.0),
                bounds.1.min(key.1),
                bounds.2.max(key.0),
                bounds.3.max(key.1),
            );
        }
        Self {
            players,
            cells,
            bounds,
            scan,
        }
    }
    fn cell(x: f64, z: f64) -> (i64, i64) {
        (
            (x / Self::CELL).floor() as i64,
            (z / Self::CELL).floor() as i64,
        )
    }
    /// The same player [`nearest_player`] finds: the nearest, the first in
    /// the list among equals.
    pub(super) fn nearest(&self, x: f64, y: f64, z: f64) -> Option<&'p LevelPlayer> {
        if self.scan || !(x.abs() < Self::FAR && z.abs() < Self::FAR) {
            return nearest_player(self.players, x, y, z);
        }
        if self.cells.is_empty() {
            return None;
        }
        let (qx, qz) = Self::cell(x, z);
        let (x0, z0, x1, z1) = self.bounds;
        let last = (qx - x0)
            .abs()
            .max((qx - x1).abs())
            .max((qz - z0).abs())
            .max((qz - z1).abs());
        let mut best: Option<(usize, f64)> = None;
        let mut looked = 0usize;
        for ring in 0..=last {
            let mut look = |kx: i64, kz: i64| {
                looked += 1;
                for &i in self.cells.get(&(kx, kz)).into_iter().flatten() {
                    let d = self.players[i].distance_sqr(x, y, z);
                    if best.is_none_or(|(b, bd)| d < bd || d == bd && i < b) {
                        best = Some((i, d));
                    }
                }
            };
            if ring == 0 {
                look(qx, qz);
            } else {
                for k in -ring..=ring {
                    look(qx + k, qz - ring);
                    look(qx + k, qz + ring);
                }
                for k in 1 - ring..ring {
                    look(qx - ring, qz + k);
                    look(qx + ring, qz + k);
                }
            }
            // Everyone in a further ring is more than `ring` cells away on one
            // axis, so further than the one found.
            let bound = ring as f64 * Self::CELL;
            if best.is_some_and(|(_, d)| d <= bound * bound) {
                break;
            }
            if looked > self.players.len() * 4 {
                return nearest_player(self.players, x, y, z);
            }
        }
        best.map(|(i, _)| &self.players[i])
    }
}

/// The game level as natural spawning reads it.
pub(super) struct GameLevel<'g, 'a> {
    pub(super) game: &'g Game<'a>,
    pub(super) w: MobWorld<'g>,
    /// Every player in the level ([`Game::level_players`]).
    pub(super) players: Vec<LevelPlayer>,
    /// Where the non-spectators among them are (`getNearestPlayer`).
    pub(super) nearest: Vec<[f64; 3]>,
    /// Boxes of the mobs this pass created, not yet in the game's list.
    pub(super) pending: std::cell::RefCell<Vec<Aabb>>,
    /// The types and boxes of what a spawner created this tick.
    pub(super) pending_ids: std::cell::RefCell<Vec<(String, Aabb)>>,
    /// The game's mobs by box, built once a pass has asked more than
    /// [`SCANNED_QUERIES`] box queries (the mobs do not change while it
    /// holds the game); the first ones scan, as the build costs a few scans.
    pub(super) boxes: std::cell::OnceCell<MobBoxes>,
    pub(super) box_queries: std::cell::Cell<u32>,
    /// The entity types some biome of the level gives a spawn cost (the
    /// nether's few): for any other the cost is none wherever it stands, and
    /// spawning asks for one for every loaded mob each tick.
    pub(super) costed: Vec<&'g str>,
}
const SCANNED_QUERIES: u32 = 8;
/// A mob not removed: its index in `Game::mobs`, box, alive, and whether
/// it is a living shulker.
type MobBox = (usize, Aabb, bool, bool);
fn mob_box(i: usize, m: &Mob) -> MobBox {
    let alive = m.alive();
    (
        i,
        m.bounds(),
        alive,
        alive && m.species() == Species::Shulker,
    )
}
/// The boxes of the game's mobs not removed, and a grid over them.
pub(super) struct MobBoxes {
    entries: Vec<MobBox>,
    grid: SeenGrid,
}
impl MobBoxes {
    fn new(mobs: &[Mob]) -> Self {
        let entries: Vec<_> = mobs
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.removed)
            .map(|(i, m)| mob_box(i, m))
            .collect();
        let grid = SeenGrid::new(entries.iter().map(|e| e.1));
        Self { entries, grid }
    }
    /// The entries whose boxes meet `area` (and possibly a few more).
    fn near(&self, area: Aabb) -> impl Iterator<Item = MobBox> + '_ {
        let mut near = Vec::new();
        let indexed = self.grid.candidates(area, &mut near);
        let all = (!indexed).then(|| self.entries.iter());
        near.into_iter()
            .map(|i| self.entries[i as usize])
            .chain(all.into_iter().flatten().copied())
    }
}
impl GameLevel<'_, '_> {
    /// The mobs not removed whose boxes may meet `area` (callers test them).
    fn mobs_near(&self, area: Aabb) -> Box<dyn Iterator<Item = MobBox> + '_> {
        let asked = self.box_queries.get() + 1;
        self.box_queries.set(asked);
        if asked <= SCANNED_QUERIES && self.boxes.get().is_none() {
            return Box::new(
                self.game
                    .mobs
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| !m.removed)
                    .map(|(i, m)| mob_box(i, m)),
            );
        }
        Box::new(
            self.boxes
                .get_or_init(|| MobBoxes::new(&self.game.mobs))
                .near(area),
        )
    }
    fn biome(&self, x: i32, y: i32, z: i32) -> Option<&BiomeInfo> {
        let chunk = self.game.world.chunks.get(&(x >> 4, z >> 4))?;
        Some(world_biomes!(self.game).get(chunk.biome(x, y, z)))
    }
    /// The block whose noise cell `BiomeManager.getBiome` reads.
    fn zoomed(&self, x: i32, y: i32, z: i32) -> [i32; 3] {
        let zoom = self.game.streaming.biome_zoom;
        let [qx, qy, qz] = zoom.quart_at_block(horizon_core::position::BlockPos::new(x, y, z));
        [qx << 2, qy << 2, qz << 2]
    }
    fn zoomed_biome(&self, x: i32, y: i32, z: i32) -> Option<&BiomeInfo> {
        let [x, y, z] = self.zoomed(x, y, z);
        self.biome(x, y, z)
    }
}
impl SpawnLevel for GameLevel<'_, '_> {
    fn state(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.w.state(x, y, z)
    }
    fn facts(&self) -> &BlockFacts {
        self.w.facts
    }
    fn mob_data(&self) -> &MobData {
        self.w.data
    }
    fn block_tag(&self, tag: &str, block: &str) -> bool {
        self.w.block_tag(tag, block)
    }
    // Level.getBiome: the zoomed biome.
    fn biome_tag(&self, tag: &str, x: i32, y: i32, z: i32) -> bool {
        let [x, y, z] = self.zoomed(x, y, z);
        self.w.biome_tag(tag, x, y, z)
    }
    fn sky_light(&self, x: i32, y: i32, z: i32) -> i32 {
        self.w.sky_light(x, y, z)
    }
    fn block_light(&self, x: i32, y: i32, z: i32) -> i32 {
        self.w.block_light(x, y, z)
    }
    fn sky_darken(&self) -> i32 {
        SpawnLevel::sky_darken(&self.w)
    }
    fn thundering(&self) -> bool {
        self.w.thundering
    }
    fn sea_level(&self) -> i32 {
        self.w.sea_level()
    }
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.w.world_surface(x, z)
    }
    fn peaceful(&self) -> bool {
        self.w.peaceful()
    }
    fn dimension(&self) -> Dimension {
        self.w.dimension()
    }
    fn surface_slime_chance(&self, x: i32, y: i32, z: i32) -> f32 {
        self.w.surface_slime_chance(x, y, z)
    }
    fn world_seed(&self) -> i64 {
        self.w.world_seed
    }
    /// `getNearestPlayer(x, y, z, distance, true)`, over every player.
    fn player_near(&self, x: f64, y: f64, z: f64, distance: f64) -> bool {
        self.players.iter().any(|p| {
            p.alive && !p.creative && !p.spectator && p.distance_sqr(x, y, z) < distance * distance
        })
    }
}
impl NaturalLevel for GameLevel<'_, '_> {
    fn spawn_hooks(&self) -> bool {
        !horizon_client::mod_api::mods().is_empty()
    }
    fn potential_spawns(&self, category: Category, pos: [i32; 3], entries: &mut Vec<SpawnEntry>) {
        let mut canceled = false;
        self.game.native_potential_spawns(
            self.game.dim.id(),
            category.name(),
            pos,
            entries,
            &mut canceled,
        );
        if canceled {
            entries.clear();
        }
    }
    fn min_y(&self) -> i32 {
        self.w.world.bottom_y()
    }
    fn max_y(&self) -> i32 {
        self.w.world.top_y() - 1
    }
    fn ambient_light(&self) -> f32 {
        match self.game.dim {
            Dim::Nether => 0.1,
            Dim::End => 0.25,
            _ => 0.0,
        }
    }
    fn has_ceiling(&self) -> bool {
        self.w.world.shape.has_ceiling
    }
    fn mob_type(&self, entity: &str) -> Option<(Species, f32, f32, Category)> {
        let t = self.game.res.mob_types().get(entity)?;
        Some((
            t.species,
            t.width as f32,
            t.height as f32,
            Category::from_name(&t.category),
        ))
    }
    // The `natural_mob_spawns` attribute reads the zoomed biome
    // (fullResolutionBiomes), the spawn probability the noise cell.
    fn biome_spawns(&self, x: i32, y: i32, z: i32) -> Option<&HashMap<String, Vec<SpawnEntry>>> {
        self.zoomed_biome(x, y, z).map(|b| &b.spawns)
    }
    fn spawn_cost(&self, entity: &str, x: i32, y: i32, z: i32) -> Option<SpawnCost> {
        if !self.costed.contains(&entity) {
            return None;
        }
        self.zoomed_biome(x, y, z)?.spawn_costs.get(entity).copied()
    }
    fn creature_probability(&self, x: i32, y: i32, z: i32) -> f32 {
        self.biome(x, y, z).map_or(0.1, |b| b.creature_probability)
    }
    fn structures_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
        wanted: &dyn Fn(&str) -> bool,
    ) -> Vec<StructureAt> {
        self.game
            .structures_reaching_where([x, y, z], wanted)
            .into_iter()
            .map(|(id, in_piece)| StructureAt {
                id,
                in_piece,
                in_bounds: true,
            })
            .collect()
    }
    fn structure_spawns(&self) -> &StructureSpawns {
        StructureSpawns::shared(&self.game.options.resources)
    }
    fn nearest_player_positions(&self) -> &[[f64; 3]] {
        &self.nearest
    }
    fn respawn(&self) -> Option<[i32; 3]> {
        // The world spawn point lives in the overworld.
        if self.game.dim != Dim::Overworld {
            return None;
        }
        self.game.level.spawn.map(|s| {
            [
                s[0].floor() as i32,
                s[1].floor() as i32,
                s[2].floor() as i32,
            ]
        })
    }
    fn can_spawn_in_chunk(&self, cx: i32, cz: i32) -> bool {
        self.game.world.chunks.contains_key(&(cx, cz))
    }
    fn no_collision(&self, bounds: Aabb) -> bool {
        let mut scene = SceneCollisions {
            world: &self.game.world,
            binding: self.w.collision,
            subject: None,
        };
        // getEntityCollisions(null, box): living shulkers are solid.
        let grown = bounds.inflate(Vec3::new(1e-7, 1e-7, 1e-7));
        no_collision_empty(&mut scene, bounds)
            && !self
                .mobs_near(grown)
                .any(|(_, b, _, shulker)| shulker && b.intersects(grown))
            && !self
                .pending_ids
                .borrow()
                .iter()
                .any(|(id, b)| id == "minecraft:shulker" && b.intersects(grown))
    }
    fn unobstructed(&self, bounds: Aabb) -> bool {
        let player = self
            .players
            .iter()
            .any(|p| p.alive && !p.spectator && p.bounds.intersects(bounds));
        !player
            && !self.pending.borrow().iter().any(|b| b.intersects(bounds))
            && !self
                .mobs_near(bounds)
                .any(|(_, b, alive, _)| alive && b.intersects(bounds))
    }
    fn spawn_mobs(&self) -> bool {
        self.game.ui.rules.bool("spawn_mobs")
    }
    /// WORLD_SURFACE (every spawn attempt's) is the chunk's kept heightmap.
    /// The others are scanned down the column: sections of air alone are
    /// skipped (air counts for no heightmap). An unloaded column reads as air.
    fn heightmap(&self, kind: Heightmap, x: i32, z: i32) -> i32 {
        let world = self.w.world;
        if kind == Heightmap::WorldSurface {
            return self.w.world_surface(x, z);
        }
        world
            .chunks
            .get(&(x >> 4, z >> 4))
            .and_then(|chunk| {
                chunk.highest(world.air.0, |y| {
                    spawn_selection::heightmap_blocks(self, kind, x, y, z)
                })
            })
            .map_or(self.w.world.bottom_y(), |y| y + 1)
    }
    fn pathfindable_land(&self, x: i32, y: i32, z: i32) -> bool {
        self.w.data.state(self.state(x, y, z)).flags & data::LAND != 0
    }
}
impl horizon_client::mobs::spawner::SpawnerLevel for GameLevel<'_, '_> {
    /// `Level.hasNearbyAlivePlayer`, over every player.
    fn alive_player_near(&self, x: f64, y: f64, z: f64, range: f64) -> bool {
        self.players.iter().any(|p| p.alive_near(x, y, z, range))
    }
    fn spawner_blocks_work(&self) -> bool {
        self.game.ui.rules.bool("spawner_blocks_work")
    }
    fn entity_exists(&self, id: &str) -> bool {
        self.game.res.mob_types().contains_key(id) || self.game.entity_type_exists(id)
    }
    fn count_entities(&self, id: &str, bounds: Aabb) -> usize {
        let mobs = self
            .mobs_near(bounds)
            .filter(|&(i, b, ..)| self.game.mobs[i].kind.id == id && b.intersects(bounds))
            .count();
        let pending = self
            .pending_ids
            .borrow()
            .iter()
            .filter(|(k, b)| k == id && b.intersects(bounds))
            .count();
        mobs + pending
    }
}
/// The live level view used by the shared vanilla finalizer. Entity ids are
/// allocated before finalization: school leaders and jockey links keep them.
struct FinalizeRuntime<'s, 'g, 'a> {
    level: &'s GameLevel<'g, 'a>,
    pending: &'s [Mob],
    rides: &'s mut Vec<(u32, u32)>,
    next_id: &'s mut u32,
}
impl FinalizeLevel for FinalizeRuntime<'_, '_, '_> {
    fn biome(&self, p: [i32; 3]) -> String {
        self.level
            .zoomed_biome(p[0], p[1], p[2])
            .map_or_else(String::new, |b| b.name.clone())
    }
    fn biome_tag(&self, tag: &str, p: [i32; 3]) -> bool {
        SpawnLevel::biome_tag(self.level, tag, p[0], p[1], p[2])
    }
    fn structures_with_piece_at(&self, p: [i32; 3]) -> Vec<String> {
        self.level
            .game
            .structures_reaching_where(p, &|_| true)
            .into_iter()
            .filter_map(|(id, in_piece)| in_piece.then_some(id))
            .collect()
    }
    fn moon_phase(&self, _: [i32; 3]) -> usize {
        self.level
            .game
            .world
            .day_time
            .div_euclid(24000)
            .rem_euclid(8) as usize
    }
    fn spawning_monsters(&self) -> bool {
        !self.level.peaceful() && self.level.game.ui.rules.bool("spawn_monsters")
    }
    fn no_collision(&self, bounds: Aabb) -> bool {
        NaturalLevel::no_collision(self.level, bounds)
    }
    fn free_chicken(&self, bounds: Aabb) -> Option<u32> {
        self.level
            .game
            .mobs
            .iter()
            .chain(self.pending)
            .find(|m| {
                m.species() == Species::Chicken
                    && m.alive()
                    && !m.removed
                    && m.extra.rider.is_none()
                    && m.bounds().intersects(bounds)
                    && !self.rides.iter().any(|&(chicken, _)| chicken == m.id)
            })
            .map(|m| m.id)
    }
    fn ride_chicken(&mut self, chicken: u32, rider: u32) {
        self.rides.push((chicken, rider));
    }
    fn game_time(&self) -> i64 {
        self.level.game.world.game_time
    }
    fn halloween(&self) -> bool {
        finalize::is_halloween()
    }
    fn create(&mut self, kind: &str, _: Reason) -> Option<Mob> {
        if self.level.peaceful() && !finalize::allowed_in_peaceful(kind) {
            return None;
        }
        let kind = self.level.game.res.mob_types().get(kind)?.clone();
        *self.next_id += 1;
        Some(Mob::new(*self.next_id, kind, Vec3::ZERO, 0.0))
    }
}

struct NaturalFinalizer {
    group: Option<Group>,
    pending: Vec<Mob>,
    rides: Vec<(u32, u32)>,
    next_id: u32,
}
impl NaturalFinalizer {
    fn new(next_id: u32) -> Self {
        Self {
            group: None,
            pending: Vec::new(),
            rides: Vec::new(),
            next_id,
        }
    }
    fn spawn<R: RandomSource>(
        &mut self,
        level: &GameLevel<'_, '_>,
        r: &SpawnRequest,
        random: &mut R,
    ) -> Option<Spawned> {
        if r.group_index == 0 {
            self.group = None;
        }
        let game = level.game;
        let kind = game.res.mob_types().get(r.entity)?.clone();
        self.next_id += 1;
        let [x, y, z] = r.position;
        let mut mob = Mob::new(self.next_id, kind, Vec3::new(x, y, z), r.yaw);
        let camel = game.res.mob_types().get("minecraft:camel_husk");
        let data = SpawnData {
            registries: &game.res.rest().spawn_registries,
            enchantments: &game.stations().enchantments,
            items: game.items(),
            camel_husk_size: camel.map_or((0.0, 0.0), |m| (m.width as f32, m.height as f32)),
        };
        let mut runtime = FinalizeRuntime {
            level,
            pending: &self.pending,
            rides: &mut self.rides,
            next_id: &mut self.next_id,
        };
        let mut cx = Ctx {
            level: &mut runtime,
            random,
            difficulty: game.local_difficulty(r.position),
            data: &data,
            created: Vec::new(),
            jockey_rider: None,
        };
        self.group = cx.finalize(&mut mob, Reason::Natural, self.group.take());
        let result = Spawned {
            bounds: mob.bounds(),
            school: !mob.extra.lone,
        };
        let added_at = self.pending.len();
        self.pending.extend(cx.created);
        self.pending.push(mob);
        // Vanilla adds the accepted mob with its passengers before another
        // attempt. Jockeys can extend outside the parent's obstruction box.
        level
            .pending
            .borrow_mut()
            .extend(self.pending[added_at..].iter().map(Mob::bounds));
        // Chickens created earlier in this pass are not in Game::mobs yet.
        for &(chicken, rider) in &self.rides {
            if let Some(m) = self.pending.iter_mut().find(|m| m.id == chicken) {
                m.extra.rider = Some(rider);
                m.extra.jockey = true;
            }
        }
        Some(result)
    }
}

/// Every entity type with a spawn cost in some biome, each once.
fn spawn_cost_types(biomes: &horizon_client::biome::Biomes) -> Vec<&str> {
    let mut types: Vec<&str> = Vec::new();
    for t in biomes.list.iter().flat_map(|b| b.spawn_costs.keys()) {
        if !types.contains(&t.as_str()) {
            types.push(t);
        }
    }
    types
}
impl<'a> Game<'a> {
    pub(super) fn natural_level(&self) -> GameLevel<'_, 'a> {
        let players = self.level_players();
        let nearest = players
            .iter()
            .filter(|p| !p.spectator)
            .map(|p| [p.position.x, p.position.y, p.position.z])
            .collect();
        GameLevel {
            game: self,
            w: self.mob_world_base(&[]),
            players,
            nearest,
            pending: Default::default(),
            pending_ids: Default::default(),
            boxes: Default::default(),
            box_queries: Default::default(),
            costed: spawn_cost_types(world_biomes!(self)),
        }
    }
    /// Adds already finalized mobs, retaining the ids their pack and riding links use.
    fn add_spawned(&mut self, pending: Vec<Mob>, rides: Vec<(u32, u32)>) {
        // Join hooks may create entities too; reserve every finalized id first.
        self.next_entity = pending
            .iter()
            .map(|m| m.id)
            .max()
            .unwrap_or(self.next_entity)
            .max(self.next_entity);
        for (chicken, rider) in rides {
            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == chicken) {
                m.extra.rider = Some(rider);
                m.extra.jockey = true;
            }
        }
        for mob in pending {
            self.mobs.push(mob);
            self.native_mobs_joined(self.mobs.len() - 1, true);
        }
    }
    /// Every player in the level in place: the local player, and the LAN
    /// guests or server players in it.
    pub(super) fn level_players(&self) -> Vec<LevelPlayer> {
        self.level_players_iter().collect()
    }
    /// `level_players` one at a time, without collecting them.
    fn level_players_iter(&self) -> impl Iterator<Item = LevelPlayer> + '_ {
        let local = self.player_view().map(|p| LevelPlayer {
            uuid: self
                .level
                .player_uuid
                .map_or(0, horizon_client::lan::playerdata::ints_uuid),
            position: p.position,
            bounds: p.bounds,
            alive: p.alive,
            creative: p.creative,
            spectator: p.spectator,
        });
        let guests = self.lan.iter().flat_map(|h| {
            h.hub
                .guests
                .iter()
                .filter(|g| h.hub.here(g))
                .map(|g| LevelPlayer {
                    uuid: g.uuid,
                    position: Vec3::new(g.position[0], g.position[1], g.position[2]),
                    bounds: super::lan_mobs::guest_bounds(g),
                    alive: !g.dead,
                    creative: g.creative(),
                    spectator: g.spectator(),
                })
        });
        local.into_iter().chain(guests)
    }
    /// `Level.hasNearbyAlivePlayer` before a level is built for the spawn
    /// code: what a spawner asks first, every tick.
    pub(super) fn alive_player_near(&self, x: f64, y: f64, z: f64, range: f64) -> bool {
        self.level_players_iter()
            .any(|p| p.alive_near(x, y, z, range))
    }
    /// `ChunkMap.collectSpawningChunks`: the natural spawn chunk tracker's
    /// chunks (within 8 chunks of a player, `getSpawnCandidateChunks`) that
    /// are ticking and within 128 blocks of a non-spectator. Also returns how
    /// many chunks the tracker holds (`getNaturalSpawnChunkCount`).
    pub(super) fn spawning_chunks(
        &self,
        players: &[LevelPlayer],
        scratch: &mut SpawnScratch,
    ) -> i32 {
        let levels = self.tickets.levels();
        scratch.collect(players, |(cx, cz)| {
            levels.loaded_ticking(cx, cz) && super::ticking_chunk(&self.world, cx, cz)
        })
    }
    /// `ServerChunkCache.tickChunks`: the spawn state over the loaded mobs,
    /// the categories under their global caps (creatures every 400 ticks),
    /// then `tickSpawningChunk` in the shuffled spawning chunks of every
    /// player: inhabited time, then `spawnForChunk`.
    pub(super) fn spawn_natural(&mut self) {
        // `tickChunks` does nothing in a Debug world.
        if self.world_debug {
            return;
        }
        let mut laps = horizon_client::spans::laps();
        let players = self.level_players();
        if players.is_empty() {
            return;
        }
        let mut scratch = std::mem::take(&mut self.spawn_scratch);
        let spawnable = self.spawning_chunks(&players, &mut scratch);
        laps.lap("sn_candidates");
        for &key in &scratch.candidates {
            *self.world.inhabited.entry(key).or_insert(0) += 1;
        }
        laps.lap("sn_inhabited");
        let spawn_mobs = self.ui.rules.bool("spawn_mobs");
        let spawn_enemies = self.ui.rules.bool("spawn_monsters");
        let mut random = std::mem::replace(&mut self.world.random, LegacyRandom::new(0));
        let mut finalizer = NaturalFinalizer::new(self.next_entity);
        let last_state;
        {
            let level = self.natural_level();
            laps.lap("sn_level");
            let counted: Vec<Counted> = self
                .mobs
                .iter()
                .map(|m| Counted {
                    entity: &m.kind.id,
                    category: Category::from_name(&m.kind.category),
                    block: [
                        m.position.x.floor() as i32,
                        m.position.y.floor() as i32,
                        m.position.z.floor() as i32,
                    ],
                    is_mob: true,
                    persistent: m.persistent
                        || horizon_client::mobs::ai::requires_custom_persistence(m),
                })
                .collect();
            let players = players
                .iter()
                .map(|p| SpawnPlayer {
                    position: [p.position.x, p.position.y, p.position.z],
                    spectator: p.spectator,
                })
                .collect();
            laps.lap("sn_materialize");
            let mut state = SpawnState::new(&level, spawnable, players, &counted, |cx, cz| {
                self.world.chunks.contains_key(&(cx, cz))
            });
            laps.lap("sn_state");
            last_state = state.counts();
            let categories = if spawn_mobs {
                state.filtered_categories(spawn_enemies, self.world.game_time % 400 == 0)
            } else {
                Vec::new()
            };
            laps.lap("sn_categories");
            // Util.shuffle.
            for i in (1..scratch.candidates.len()).rev() {
                let j = random.next_int_bound(i as i32 + 1).unwrap_or(0) as usize;
                scratch.candidates.swap(i, j);
            }
            laps.lap("sn_shuffle");
            spawn_selection::attempt_counters(false);
            if !categories.is_empty() {
                for chunk in scratch.candidates.drain(..) {
                    let mut spawn = |l: &GameLevel, r: &SpawnRequest, random: &mut LegacyRandom| {
                        finalizer.spawn(l, r, random)
                    };
                    spawn_selection::spawn_for_chunk_with_random(
                        &level,
                        &mut random,
                        &mut state,
                        &categories,
                        chunk,
                        &mut spawn,
                        &mut |_| {},
                    );
                }
            }
        }
        spawn_selection::attempt_counters(true);
        laps.lap("sn_attempts");
        self.world.random = random;
        // ServerChunkCache.lastSpawnState, for the debug overlay.
        self.spawn_counts.insert(self.dim, last_state);
        self.add_spawned(finalizer.pending, finalizer.rides);
        self.spawn_scratch = scratch;
        laps.lap("sn_finalize");
    }
    /// `NoiseBasedChunkGenerator.spawnOriginalMobs`: creature packs for a
    /// freshly generated chunk, from the biome at its corner column's top,
    /// with the chunk's decoration-seeded random, finalized with the
    /// region's random as generation adds them. The settings' own
    /// `disable_mob_generation` skips the pass, which is how the End's preset
    /// keeps its chunks empty (the dimension is also checked, as before, so a
    /// preset that fails to load cannot start filling the End).
    pub(super) fn spawn_generation_creatures(&mut self, cx: i32, cz: i32) {
        if !self.world.chunks.contains_key(&(cx, cz)) || self.dim == Dim::End {
            return;
        }
        let Some(terrain) = self.end.locator() else {
            return;
        };
        // `FlatLevelSource.spawnOriginalMobs` is empty: a superflat world places no packs.
        if terrain.is_flat() {
            return;
        }
        // `NoiseGeneratorSettings.disableMobGeneration`: the shipped End preset
        // asks for no packs at generation, and so would any dimension using it.
        if terrain.disable_mob_generation() {
            return;
        }
        let mut region = terrain.region_random(cx, cz);
        let any = region.as_any();
        let added = if let Some(r) = any.downcast_mut::<LegacyRandom>() {
            self.generation_creatures(cx, cz, &terrain, r)
        } else if let Some(r) = any.downcast_mut::<XoroshiroRandom>() {
            self.generation_creatures(cx, cz, &terrain, r)
        } else {
            return;
        };
        for nbt in added {
            self.load_mob(Tag::Compound(nbt), None);
        }
    }
    fn generation_creatures<Q: RandomSource + 'static>(
        &self,
        cx: i32,
        cz: i32,
        terrain: &VanillaTerrain,
        region: &mut Q,
    ) -> Vec<Compound> {
        let mut random = WorldgenRandom::new(LegacyRandom::new(0));
        random.set_decoration_seed(self.level.seed, cx * 16, cz * 16);
        let mut added = Vec::new();
        let mut group = None;
        let level = self.natural_level();
        let source = [cx * 16, level.max_y(), cz * 16];
        let mut spawn = |l: &GameLevel, r: &SpawnRequest, region: &mut Q| {
            // Each pack starts without group data.
            if r.group_index == 0 {
                group = None;
            }
            let [x, y, z] = r.position;
            let (bx, by, bz) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
            let biome = l.zoomed_biome(bx, by, bz).map_or("", |b| b.name.as_str());
            let at = Vec3::new(x, y, z);
            match terrain.spawn_generated(r.entity, at, r.yaw, biome, region, &mut group) {
                Some(nbt) => {
                    added.push(nbt);
                    true
                }
                None => false,
            }
        };
        spawn_selection::spawn_mobs_for_chunk_generation(
            &level,
            source,
            (cx, cz),
            &mut random,
            region,
            &mut spawn,
            &mut |_| {},
        );
        added
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These age and level-RNG results come from VanillaFinalizeSpawnOracle:
    /// cow seed=0 pack=4 and tropical_fish seed=5 pack=1, NATURAL, plains.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn runtime_natural_finalizer_preserves_pack_data_and_level_random() {
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            root.clone(),
            None,
            &root,
            &settings,
            std::time::Instant::now(),
            false,
            true,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = super::super::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&root),
            None,
        )
        .unwrap();
        let plains = game
            .res
            .biomes()
            .list
            .iter()
            .position(|b| b.name == "minecraft:plains")
            .unwrap() as u16;
        for cx in -1..=0 {
            for cz in -1..=0 {
                let mut chunk = horizon_client::chunk::Chunk::empty(cx, cz);
                for qx in 0..4 {
                    for qz in 0..4 {
                        for qy in -16..80 {
                            chunk.set_biome_cell(qx, qy, qz, plains);
                        }
                    }
                }
                game.world
                    .chunks
                    .insert((cx, cz), std::sync::Arc::new(chunk));
            }
        }
        let mut finalizer = NaturalFinalizer::new(game.next_entity);
        let mut random = LegacyRandom::new(0);
        {
            let level = game.natural_level();
            let mut request = SpawnRequest {
                entity: "minecraft:cow",
                species: Species::Cow,
                position: [0.5, 64.0, 0.5],
                yaw: 0.0,
                group_index: 0,
                reason: Reason::Natural,
            };
            for i in 0..4 {
                request.group_index = i;
                finalizer.spawn(&level, &request, &mut random).unwrap();
            }
            assert_eq!(
                finalizer
                    .pending
                    .iter()
                    .map(|m| m.extra.age)
                    .collect::<Vec<_>>(),
                vec![0, 0, -24000, 0]
            );
            assert_eq!(random.next_long(), -8062155292093192501);
            // The first member of another pack cannot inherit the old size.
            request.group_index = 0;
            finalizer.spawn(&level, &request, &mut random).unwrap();
            assert_eq!(finalizer.pending.last().unwrap().extra.age, 0);
            assert!(matches!(&finalizer.group, Some(Group::Ageable(a)) if a.size == 1));
            request.entity = "minecraft:tropical_fish";
            request.species = Species::TropicalFish;
            random = LegacyRandom::new(5);
            let result = finalizer.spawn(&level, &request, &mut random).unwrap();
            assert!(!result.school, "a loner ends the tropical-fish pack");
            assert_eq!(random.next_long(), 7887083129949472399);
            // This seed creates a skeleton rider in the existing Java oracle
            // corpus (case/15613). Spider and skeleton draws are the same for
            // its TRIAL_SPAWNER reason and this NATURAL reason, on normal.
            request.entity = "minecraft:spider";
            request.species = Species::Spider;
            request.position = [8.5, 64.0, 8.5];
            random = LegacyRandom::new(4122790947106936923);
            let parent = finalizer.spawn(&level, &request, &mut random).unwrap();
            let spider = finalizer.pending.last().unwrap();
            let skeleton = finalizer
                .pending
                .iter()
                .find(|m| m.extra.mount == Some(spider.id))
                .unwrap();
            assert_eq!(skeleton.species(), Species::Skeleton);
            assert_eq!(spider.extra.rider, Some(skeleton.id));
            assert_eq!(skeleton.extra.main_hand.as_deref(), Some("minecraft:bow"));
            let rider_only = Aabb::new(Vec3::new(8.45, 65.1, 8.45), Vec3::new(8.55, 65.2, 8.55));
            assert!(!parent.bounds.intersects(rider_only));
            assert!(skeleton.bounds().intersects(rider_only));
            assert!(
                !level.unobstructed(rider_only),
                "the rider obstructs the next attempt"
            );
            assert_eq!(
                level.pending.borrow().len(),
                finalizer.pending.len(),
                "register each accepted entity once"
            );
        }
        // Insertion must preserve finalized equipment, randoms and riding links.
        let finalized: Vec<_> = finalizer
            .pending
            .iter()
            .map(|m| {
                (
                    m.id,
                    (0..6)
                        .map(|slot| finalize::item(m, slot))
                        .collect::<Vec<_>>(),
                    m.extra.gear.clone(),
                    m.random.clone().next_long(),
                    m.extra.mount,
                    m.extra.rider,
                )
            })
            .collect();
        let ids: Vec<_> = finalizer.pending.iter().map(|m| m.id).collect();
        game.world.random = LegacyRandom::new(2468);
        let expected_level_draw = game.world.random.clone().next_long();
        insert_finalized(&mut game, finalizer.pending, finalizer.rides);
        assert_eq!(
            game.world.random.next_long(),
            expected_level_draw,
            "insertion must not repeat spawn draws"
        );
        assert_eq!(game.mobs.iter().map(|m| m.id).collect::<Vec<_>>(), ids);
        assert_eq!(game.next_entity, *ids.iter().max().unwrap());
        let inserted: Vec<_> = game
            .mobs
            .iter()
            .map(|m| {
                (
                    m.id,
                    (0..6)
                        .map(|slot| finalize::item(m, slot))
                        .collect::<Vec<_>>(),
                    m.extra.gear.clone(),
                    m.random.clone().next_long(),
                    m.extra.mount,
                    m.extra.rider,
                )
            })
            .collect();
        assert_eq!(inserted, finalized);
    }

    fn insert_finalized(game: &mut Game<'_>, pending: Vec<Mob>, rides: Vec<(u32, u32)>) {
        game.add_spawned(pending, rides);
    }

    fn player(x: f64, y: f64, z: f64, spectator: bool) -> LevelPlayer {
        LevelPlayer {
            uuid: 0,
            position: Vec3::new(x, y, z),
            bounds: Aabb::new(Vec3::new(x, y, z), Vec3::new(x, y, z)),
            alive: true,
            creative: false,
            spectator,
        }
    }

    /// The candidates as the uncached port collected them every tick: each
    /// player's square in dx/dz order into one ordered set, close to any
    /// player that holds the chunk. Returns the set's size too.
    fn uncached_candidates(players: &[LevelPlayer]) -> (i32, Vec<(i32, i32)>) {
        let mut order = Vec::new();
        let mut close = Vec::new();
        let mut seen = HashMap::new();
        for p in players {
            let (pcx, pcz) = SpawnScratch::chunk(p);
            for dx in -8..=8 {
                for dz in -8..=8 {
                    let key = (pcx + dx, pcz + dz);
                    let i = *seen.entry(key).or_insert_with(|| {
                        order.push(key);
                        close.push(false);
                        order.len() - 1
                    });
                    close[i] |= close_enough_for_spawning(p, key);
                }
            }
        }
        let kept = order.iter().zip(&close).filter(|c| *c.1).map(|c| *c.0);
        (order.len() as i32, kept.collect())
    }

    #[test]
    fn spawn_candidates_keep_order_and_live_proximity_across_cache_hits() {
        let mut players = vec![
            player(0.0, 64.0, 0.0, false),
            player(16.0, 64.0, 0.0, false),
        ];
        let mut scratch = SpawnScratch::default();
        // Two adjacent 17x17 squares: the first square, then the new east edge.
        assert_eq!(scratch.collect(&players, |_| true), 306);
        assert!(scratch.candidates.contains(&(-8, 0)));
        // Spawning consumes the working list; the next pass must not care.
        scratch.candidates.reverse();
        scratch.candidates.truncate(3);
        // Moving inside its chunk takes player 0 out of range of the west edge.
        players[0].position.x = 15.5;
        assert_eq!(scratch.collect(&players, |_| true), 306);
        assert!(!scratch.candidates.contains(&(-8, 0)));
        assert_eq!(scratch.candidates, uncached_candidates(&players).1);
        // Nothing is close to spectators, though the squares still count.
        for p in &mut players {
            p.spectator = true;
        }
        assert_eq!(scratch.collect(&players, |_| true), 306);
        assert!(scratch.candidates.is_empty());
        for p in &mut players {
            p.spectator = false;
        }
        // Player 1 near its chunk's east side reaches the middle of chunk 9.
        players[1].position.x = 31.9;
        assert_eq!(scratch.collect(&players, |_| true), 306);
        assert!(scratch.candidates.contains(&(9, 0)));
        assert_eq!(scratch.candidates, uncached_candidates(&players).1);
        // `keep` filters without reordering.
        scratch.collect(&players, |(x, z)| x == z);
        let diagonal: Vec<_> = uncached_candidates(&players)
            .1
            .into_iter()
            .filter(|&(x, z)| x == z)
            .collect();
        assert!(diagonal.len() > 1);
        assert_eq!(scratch.candidates, diagonal);
    }

    /// Players walking, crossing borders, meeting, splitting, joining, leaving
    /// and swapping order: every pass matches the uncached collection.
    #[test]
    fn spawn_candidates_match_the_uncached_union_while_players_move() {
        let mut seed = 0x5DEECE66Du64;
        let mut next = move |bound: u64| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) % bound
        };
        let mut players: Vec<LevelPlayer> = [0.0, 40.0, 300.0, 1200.0, -500.0]
            .iter()
            .map(|&x| player(x, 64.0, x / 2.0, false))
            .collect();
        let mut scratch = SpawnScratch::default();
        for step in 0..3000 {
            match next(40) {
                0 if players.len() < 8 => {
                    let x = next(1200) as f64 - 600.0;
                    let at = next(players.len() as u64 + 1) as usize;
                    players.insert(at, player(x, 64.0, -x, false));
                }
                1 if players.len() > 1 => {
                    players.remove(next(players.len() as u64) as usize);
                }
                2 if players.len() > 1 => {
                    let i = next(players.len() as u64 - 1) as usize;
                    players.swap(i, i + 1);
                }
                3 => {
                    let i = next(players.len() as u64) as usize;
                    players[i].spectator = !players[i].spectator;
                }
                4 => {
                    // Teleport onto another player: identical squares.
                    let (i, j) = (next(players.len() as u64), next(players.len() as u64));
                    players[i as usize].position = players[j as usize].position;
                }
                _ => {
                    for p in &mut players {
                        p.position.x += next(13) as f64 - 6.0 + 0.37;
                        p.position.z += next(13) as f64 - 6.0 - 0.21;
                    }
                }
            }
            let (count, expected) = uncached_candidates(&players);
            assert_eq!(scratch.collect(&players, |_| true), count, "step {step}");
            assert_eq!(scratch.candidates, expected, "step {step}");
        }
    }

    /// The grid finds the same player a look at every one finds, the first of
    /// two at the same distance included, and never a spectator.
    #[test]
    fn nearest_players_find_what_a_scan_finds() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = |m: f64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 1_000_000) as f64 / 1_000_000.0 * 2.0 * m - m
        };
        let mut players: Vec<LevelPlayer> = (0..150)
            .map(|i| player(next(3_000.0), 64.0 + next(60.0), next(3_000.0), i % 11 == 0))
            .collect();
        // Two at the same distance from the origin, the second one first in no cell order.
        players.push(player(300.0, 64.0, 0.0, false));
        players.push(player(-300.0, 64.0, 0.0, false));
        players.push(player(0.0, 64.0, 300.0, false));
        let grid = NearestPlayers::new(&players);
        let index = |p: Option<&LevelPlayer>| p.map(|p| std::ptr::from_ref(p) as usize);
        let mut probes: Vec<(f64, f64, f64)> = (0..400)
            .map(|_| (next(3_000.0), next(100.0), next(3_000.0)))
            .collect();
        probes.extend([(0.0, 64.0, 0.0), (1.0e12, 0.0, 0.0), (f64::NAN, 0.0, 0.0)]);
        for (x, y, z) in probes {
            assert_eq!(
                index(grid.nearest(x, y, z)),
                index(nearest_player(&players, x, y, z)),
                "({x}, {y}, {z})"
            );
        }
        players.push(player(NearestPlayers::FAR * 2.0, 64.0, 0.0, false));
        let grid = NearestPlayers::new(&players);
        assert!(grid.scan);
        for x in [0.0, NearestPlayers::FAR * 2.0] {
            assert_eq!(
                index(grid.nearest(x, 64.0, 0.0)),
                index(nearest_player(&players, x, 64.0, 0.0))
            );
        }
        let spectators: Vec<LevelPlayer> = (0..20)
            .map(|i| player(f64::from(i) * 100.0, 0.0, 0.0, true))
            .collect();
        assert!(
            NearestPlayers::new(&spectators)
                .nearest(0.0, 0.0, 0.0)
                .is_none()
        );
    }
}
