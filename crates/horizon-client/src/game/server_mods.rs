//! Server mods in the game (`docs/mods.md`, "Server mods"): the game that
//! simulates the world (in-frame single player, the server thread, a LAN
//! host, the dedicated server) runs them. At the end of each tick it tells
//! them who joined and left, then what players said, broke and placed since
//! the last one, then ticks them; a player's command reaches them at once.
//! Their lines go to the players they name. With no server mod on, each hook
//! here is one check.
use super::{Game, lan::LanHost};
#[cfg(not(feature = "mods"))]
use horizon_world::registry::BlockStateId;

/// Who did something a server mod hears about, resolved at the tick's end.
#[cfg_attr(not(feature = "mods"), allow(dead_code))]
pub(super) enum Who {
    /// The window's own player, in-frame.
    Local,
    /// A connection's player (the server thread's owner is one).
    Conn(u32),
    Uuid(u128),
}

/// What players did during a tick, for the server mods at its end.
#[cfg_attr(not(feature = "mods"), allow(dead_code))]
pub(super) enum ModEvent {
    Chat(Who, String),
    Block {
        who: Who,
        pos: [i32; 3],
        state: String,
        placed: bool,
    },
}

#[cfg(not(feature = "mods"))]
impl Game<'_> {
    pub(super) fn tick_server_mods(&mut self) {}
    pub(super) fn mod_block_event(&mut self, _pos: [i32; 3], _state: BlockStateId, _placed: bool) {}
    pub(super) fn mod_player_chat(&mut self, _who: Who, _text: &str) {}
    pub(super) fn local_server_mod_command(&mut self, _line: &str) -> bool {
        false
    }
    pub(super) fn guest_server_mod_command(
        &mut self,
        _host: &mut LanHost,
        _conn: u32,
        _line: &str,
    ) -> bool {
        false
    }
    pub(super) fn apply_mod_block_writes(&mut self) {}
}

#[cfg(feature = "mods")]
mod run {
    use super::{Game, LanHost, ModEvent, Who};
    use horizon_client::chunk_map::ChunkMap;
    use horizon_client::{
        command::args::GameMode as Mode,
        lan::data::component_nbt,
        text::{self, Text},
    };
    use horizon_core::{
        geometry::{Aabb, Vec3, VoxelShape, try_resolve_step_movement},
        position::{BlockPos as NativeBlockPos, ChunkPos},
    };
    use horizon_mods::{
        ArgumentKind, BlockPos, EntityMovementDelta, EntityMovementResult, EntityState, EntityType,
        FluidContext, GameMode, LoadedEnvironment, Output, ServerCommand, ServerPlayer,
        StaticCollision, WorldEnvironment, WorldView,
    };
    use horizon_protocol::packets::play::{self, Clientbound as C};
    use horizon_world::registry::{BlockRegistry, BlockStateId};
    use horizon_world::{
        block_collision::BlockCollisionRegistry,
        block_collision_scan::{BlockCollisionScan, BlockCollisionWorld, CollisionBlock},
        collision_context::CollisionContext,
        fluid::FluidState,
        heightmap::HeightmapType,
        worldgen::terrain::TerrainBlockStates,
    };
    use serde_json::{Value, json};

    struct SnapshotAdmission(std::sync::Arc<horizon_client::save::Persistence>);
    impl horizon_mods::WorldStorageAdmission for SnapshotAdmission {
        fn write(&self, operation: &mut dyn FnMut() -> Result<(), String>) -> Result<(), String> {
            self.0
                .admit_write(|| operation().map_err(Into::into))
                .map_err(|e| e.to_string())
        }
    }

    /// Blocks queued for levels that don't tick (another dimension, in-frame)
    /// are kept up to this many; the oldest go first.
    const MAX_QUEUED_WRITES: usize = 1 << 16;

    /// Resident chunks lent for world queries and entity collision during a callback.
    struct LentWorld {
        registry: &'static BlockRegistry,
        biomes: std::sync::Arc<horizon_client::biome::Biomes>,
        dims: Vec<(&'static str, ChunkMap)>,
        resources: std::sync::Arc<super::super::Resources>,
        heights: Vec<(&'static str, i32, i32)>,
        properties: horizon_client::assets::Properties,
        clock: (i64, i64),
        weather: horizon_client::world::Weather,
    }
    const COLLISION_CELLS: usize = 4096;

    struct LentCollisions<'a> {
        chunks: &'a ChunkMap,
        binding: &'a BlockCollisionRegistry<'static>,
        min_y: i32,
        max_y: i32,
    }
    impl LentCollisions<'_> {
        fn collect(&mut self, bounds: Aabb) -> Result<Vec<VoxelShape>, String> {
            let lo = bounds.min_position();
            let hi = bounds.max_position();
            if !lo.is_finite()
                || !hi.is_finite()
                || lo.x < -29_999_984.0
                || lo.z < -29_999_984.0
                || hi.x > 29_999_984.0
                || hi.z > 29_999_984.0
            {
                return Err("entity collision bounds are outside the supported world".into());
            }
            let start = [
                (lo.x - 1e-7).floor() as i32 - 1,
                (lo.y - 1e-7).floor() as i32 - 1,
                (lo.z - 1e-7).floor() as i32 - 1,
            ];
            let end = [
                (hi.x + 1e-7).floor() as i32 + 1,
                (hi.y + 1e-7).floor() as i32 + 1,
                (hi.z + 1e-7).floor() as i32 + 1,
            ];
            if start[1] < self.min_y || end[1] > self.max_y {
                return Err("entity collision scan crosses the dimension height boundary".into());
            }
            let cells = (0..3).try_fold(1usize, |n, i| {
                let size = usize::try_from(end[i] - start[i] + 1).ok()?;
                n.checked_mul(size).filter(|n| *n <= COLLISION_CELLS)
            });
            if cells.is_none() {
                return Err("entity collision scan exceeds 4096 candidate cells".into());
            }
            for x in (start[0] >> 4)..=(end[0] >> 4) {
                for z in (start[2] >> 4)..=(end[2] >> 4) {
                    if !self.chunks.contains_key(&(x, z)) {
                        return Err("entity collision scan reaches an unloaded chunk".into());
                    }
                }
            }
            BlockCollisionScan::new(
                self,
                CollisionContext::position(lo.y),
                bounds,
                false,
                COLLISION_CELLS,
            )
            .map_err(|e| e.to_string())?
            .map(|hit| hit.map(|hit| hit.shape).map_err(|e| e.to_string()))
            .collect()
        }
    }
    impl BlockCollisionWorld for LentCollisions<'_> {
        type Error = String;
        type Chunk = ChunkPos;
        fn collision_chunk(&mut self, p: ChunkPos) -> Result<Option<ChunkPos>, String> {
            if !self.chunks.contains_key(&(p.x, p.z)) {
                return Err("entity collision scan reaches an unloaded chunk".into());
            }
            Ok(Some(p))
        }
        fn collision_block(
            &mut self,
            _: ChunkPos,
            p: NativeBlockPos,
        ) -> Result<CollisionBlock, String> {
            if p.y < self.min_y || p.y > self.max_y {
                return Err("entity collision scan crosses the dimension height boundary".into());
            }
            let chunk = self
                .chunks
                .get(&(p.x >> 4, p.z >> 4))
                .ok_or("entity collision scan reaches an unloaded chunk")?;
            let state = chunk.block(p.x & 15, p.y, p.z & 15);
            let description = self
                .binding
                .registry()
                .state(state)
                .ok_or("entity collision state is outside the registry")?;
            if matches!(
                description.name.as_str(),
                "minecraft:water"
                    | "minecraft:lava"
                    | "minecraft:powder_snow"
                    | "minecraft:scaffolding"
            ) || self
                .binding
                .cached_shapes()
                .cached_shape(state)
                .map_err(|e| e.to_string())?
                .is_none()
            {
                return Err(format!(
                    "entity block collision context is unsupported for {}",
                    description.name
                ));
            }
            self.binding
                .collision_block(state)
                .map_err(|e| e.to_string())
        }
        fn is_suffocating(
            &mut self,
            _: ChunkPos,
            _: CollisionBlock,
            _: NativeBlockPos,
        ) -> Result<bool, String> {
            Err("entity collision suffocation queries are unsupported".into())
        }
        fn context_shape(
            &mut self,
            block: CollisionBlock,
            _: NativeBlockPos,
            _: &CollisionContext<'_>,
        ) -> Result<VoxelShape, String> {
            self.binding
                .cached_shapes()
                .cached_shape(block.state)
                .map_err(|e| e.to_string())?
                .cloned()
                .ok_or_else(|| "entity dynamic block collision is unsupported".into())
        }
    }
    impl WorldView for LentWorld {
        fn sky_visible(
            &self,
            dimension: &str,
            x: i32,
            y: i32,
            z: i32,
        ) -> Result<Option<bool>, String> {
            let Some((_, chunks)) = self.dims.iter().find(|(d, _)| *d == dimension) else {
                return Ok(None);
            };
            let Some((_, min_y, max_y)) = self.heights.iter().find(|(d, _, _)| *d == dimension)
            else {
                return Ok(None);
            };
            if y < *min_y || y > *max_y {
                return Ok(None);
            }
            let Some(chunk) = chunks.get(&(x >> 4, z >> 4)) else {
                return Ok(None);
            };
            if !chunk.lit {
                return Ok(None);
            }
            // Same Level.canSeeSky semantics as the native mob world.
            Ok(Some(
                dimension != "minecraft:the_nether" && chunk.light(x & 15, y, z & 15) >> 4 == 15,
            ))
        }
        fn environment(
            &self,
            dimension: &str,
            x: i32,
            y: i32,
            z: i32,
        ) -> Result<Option<WorldEnvironment>, String> {
            let Some((_, chunks)) = self.dims.iter().find(|(d, _)| *d == dimension) else {
                return Ok(None);
            };
            let (rain_level, thunder_level) = self.weather.levels(1.0);
            let mut result = WorldEnvironment {
                game_time: self.clock.0,
                day_time: self.clock.1,
                raining: self.weather.raining,
                thundering: self.weather.thundering,
                rain_level,
                thunder_level,
                loaded: None,
            };
            let Some((_, min_y, max_y)) = self.heights.iter().find(|(d, _, _)| *d == dimension)
            else {
                return Ok(Some(result));
            };
            if y < *min_y || y > *max_y {
                return Ok(Some(result));
            }
            let Some(chunk) = chunks.get(&(x >> 4, z >> 4)) else {
                return Ok(Some(result));
            };
            let state = chunk.block(x & 15, y, z & 15);
            let description = self
                .registry
                .state(state)
                .ok_or("environment block state is outside the native registry")?;
            let biome = self
                .biomes
                .list
                .get(chunk.biome(x, y, z) as usize)
                .ok_or("environment biome is outside the native registry")?;
            let light = chunk.lit.then(|| chunk.light(x & 15, y, z & 15));
            let sky_light = light.map(|light| light >> 4);
            let block_light = light.map(|light| light & 15);
            // One native heightmap column, avoiding full-chunk cache priming.
            let mut surface = horizon_client::chunk::MIN_Y;
            for sy in (horizon_client::chunk::MIN_Y
                ..horizon_client::chunk::MIN_Y + horizon_client::chunk::HEIGHT)
                .rev()
            {
                let block = chunk.block(x & 15, sy, z & 15);
                if self.registry.state(block).is_none() {
                    return Err("environment column state is outside the native registry".into());
                }
                if HeightmapType::MotionBlocking.is_opaque(self.properties.heightmap_state(block)) {
                    surface = sy + 1;
                    break;
                }
            }
            let rain_at_position =
                if rain_level <= 0.2 || surface > y || biome.snows_at(x, y, z) != Some(false) {
                    Some(false)
                } else {
                    sky_light.map(|sky| sky == 15)
                };
            let static_collision = if matches!(
                description.name.as_str(),
                "minecraft:water"
                    | "minecraft:lava"
                    | "minecraft:scaffolding"
                    | "minecraft:powder_snow"
            ) {
                None
            } else {
                self.resources
                    .collision()
                    .cached_shapes()
                    .cached_shape(state)
                    .map_err(|e| e.to_string())?
                    .map(|shape| {
                        if shape.is_empty() {
                            StaticCollision::Empty
                        } else if shape.is_block() {
                            StaticCollision::FullBlock
                        } else {
                            StaticCollision::Partial
                        }
                    })
            };
            let fluid = if description.name.starts_with("minecraft:")
                || description.properties.contains_key("waterlogged")
            {
                FluidState::from_block(description)
                    .ok()
                    .map(|fluid| FluidContext {
                        kind: fluid.kind().name().into(),
                        amount: fluid.amount(),
                        falling: fluid.falling(),
                    })
            } else {
                None
            };
            result.loaded = Some(LoadedEnvironment {
                block: state_string(self.registry, state),
                biome: biome.name.clone(),
                sky_light,
                block_light,
                static_collision,
                fluid,
                surface_y: Some(surface),
                rain_at_position,
            });
            Ok(Some(result))
        }
        fn move_colliding(
            &self,
            state: &EntityState,
            kind: &EntityType,
            delta: &EntityMovementDelta,
        ) -> Result<EntityMovementResult, String> {
            let (_, chunks) = self
                .dims
                .iter()
                .find(|(d, _)| *d == state.dimension)
                .ok_or("entity collision dimension is not loaded")?;
            let (_, min_y, max_y) = self
                .heights
                .iter()
                .find(|(d, _, _)| *d == state.dimension)
                .ok_or("entity collision dimension height is unavailable")?;
            let p = &state.transform;
            let half = f64::from(kind.width) * 0.5;
            let bounds = Aabb::new(
                Vec3::new(p.x - half, p.y, p.z - half),
                Vec3::new(p.x + half, p.y + f64::from(kind.height), p.z + half),
            );
            let mut world = LentCollisions {
                chunks,
                binding: self.resources.collision(),
                min_y: *min_y,
                max_y: *max_y,
            };
            if !world.collect(bounds)?.is_empty() {
                return Err("entity collision body starts inside a block".into());
            }
            let movement = Vec3::new(delta.x, delta.y, delta.z);
            let moved = try_resolve_step_movement(movement, bounds, 0.0, false, |bounds| {
                world.collect(bounds)
            })?;
            let supported = try_resolve_step_movement(
                Vec3::new(0.0, -1e-5, 0.0),
                bounds.move_by(moved),
                0.0,
                false,
                |bounds| world.collect(bounds),
            )?;
            Ok(EntityMovementResult {
                x: p.x + moved.x,
                y: p.y + moved.y,
                z: p.z + moved.z,
                on_ground: supported.y > -1e-5,
            })
        }
        fn block(&self, dimension: &str, x: i32, y: i32, z: i32) -> Option<String> {
            let (_, chunks) = self.dims.iter().find(|(d, _)| *d == dimension)?;
            let chunk = chunks.get(&(x >> 4, z >> 4))?;
            Some(state_string(self.registry, chunk.block(x & 15, y, z & 15)))
        }
        fn biome(&self, dimension: &str, x: i32, y: i32, z: i32) -> Option<String> {
            let (_, chunks) = self.dims.iter().find(|(d, _)| *d == dimension)?;
            let chunk = chunks.get(&(x >> 4, z >> 4))?;
            Some(self.biomes.get(chunk.biome(x, y, z)).name.clone())
        }
        fn has_dimension(&self, dimension: &str) -> bool {
            matches!(
                dimension,
                "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
            )
        }
        fn check_state(&self, state: &str) -> Result<(), String> {
            crate::game::render_control::parse_state(self.registry, state)
                .map(drop)
                .map_err(|e| format!("{state}: {e}"))
        }
    }

    /// Server mods' commands as a root of the commands report
    /// (`lan::commands`): a literal per command, then its arguments in a
    /// chain, each node runnable where only optional arguments follow.
    fn command_tree<'a>(commands: impl Iterator<Item = &'a ServerCommand>) -> Value {
        let mut root = serde_json::Map::new();
        for c in commands {
            let mut next: Option<(String, Value)> = None;
            for (i, a) in c.arguments.iter().enumerate().rev() {
                let executable = c.arguments.get(i + 1).is_none_or(|b| b.optional);
                let (parser, properties) = match a.kind {
                    ArgumentKind::Word => ("brigadier:string", json!({"type": "word"})),
                    ArgumentKind::Text => ("brigadier:string", json!({"type": "greedy"})),
                    ArgumentKind::Integer((min, max)) => {
                        let mut p = serde_json::Map::new();
                        if let Some(min) = min {
                            p.insert("min".into(), min.into());
                        }
                        if let Some(max) = max {
                            p.insert("max".into(), max.into());
                        }
                        ("brigadier:integer", Value::Object(p))
                    }
                    ArgumentKind::Number => ("brigadier:double", Value::Null),
                    ArgumentKind::Boolean => ("brigadier:bool", Value::Null),
                    ArgumentKind::Player => (
                        "minecraft:entity",
                        json!({"amount": "single", "type": "players"}),
                    ),
                    ArgumentKind::BlockPos => ("minecraft:block_pos", Value::Null),
                };
                let mut node = json!({
                    "type": "argument",
                    "parser": parser,
                    "properties": properties,
                    "executable": executable,
                });
                if let Some((name, child)) = next.take() {
                    node["children"] = json!({ name: child });
                }
                next = Some((a.name.clone(), node));
            }
            let executable = c.arguments.first().is_none_or(|a| a.optional);
            let mut literal = json!({"type": "literal", "executable": executable});
            if let Some((name, child)) = next {
                literal["children"] = json!({ name: child });
            }
            root.insert(c.name.clone(), literal);
        }
        if root.is_empty() {
            return Value::Null;
        }
        json!({"type": "root", "children": root})
    }

    /// `minecraft:oak_log[axis=y]`.
    fn state_string(registry: &BlockRegistry, state: BlockStateId) -> String {
        let Some(s) = registry.state(state) else {
            return "minecraft:air".into();
        };
        if s.properties.is_empty() {
            return s.name.clone();
        }
        let props: Vec<String> = s
            .properties
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        format!("{}[{}]", s.name, props.join(","))
    }

    /// `UUID.toString()`.
    fn uuid_string(u: u128) -> String {
        let h = format!("{u:032x}");
        format!(
            "{}-{}-{}-{}-{}",
            &h[..8],
            &h[8..12],
            &h[12..16],
            &h[16..20],
            &h[20..]
        )
    }

    fn mode(id: i32) -> GameMode {
        match id {
            1 => GameMode::Creative,
            2 => GameMode::Adventure,
            3 => GameMode::Spectator,
            _ => GameMode::Survival,
        }
    }

    fn ray_record(
        origin: [f64; 3],
        direction: [f64; 3],
        limit: f64,
        record: &horizon_mods::entities::Record,
    ) -> Option<f64> {
        let t = &record.state.transform;
        let half = f64::from(record.kind.width) * 0.5;
        super::super::ray_box(
            origin,
            direction,
            limit,
            Aabb::new(
                Vec3::new(t.x - half, t.y, t.z - half),
                Vec3::new(t.x + half, t.y + f64::from(record.kind.height), t.z + half),
            ),
        )
    }
    impl Game<'_> {
        pub(in crate::game) fn mod_entity_targeted(&self) -> bool {
            self.mod_entity_target().is_some()
        }
        fn mod_entity_target(&self) -> Option<String> {
            if !self.hears_players() || !self.mods.mods.wants_interactions() {
                return None;
            }
            let origin = self.eye_position()?;
            let direction = super::super::view_vector(self.yaw, self.pitch);
            let reach = if self.creative { 5.0 } else { 3.0 };
            let (distance, _, handle) = self
                .mod_entities
                .records()
                .into_iter()
                .filter(|r| r.state.dimension == self.dim.id())
                .filter_map(|r| {
                    ray_record(origin, direction, reach, &r).map(|d| (d, r.engine_id, r.handle))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))?;
            self.mod_hit_before_native(distance).then_some(handle)
        }
        /// Both in-frame use and as_guest arrive with the authoritative actor swapped in.
        pub(in crate::game) fn mod_use_interaction(
            &mut self,
            hit: Option<&super::super::Hit>,
            block_phase: bool,
        ) -> bool {
            use horizon_mods::{BlockUse, InteractionHand, InteractionPos, ItemUse};
            if !self.hears_players() || !self.mods.mods.wants_interactions() {
                return false;
            }
            if !block_phase && matches!(self.guest_use, Some(super::super::GuestUse::OnBlock(_))) {
                return false;
            }
            let Some(origin) = self.eye_position() else {
                return false;
            };
            let direction = super::super::view_vector(self.yaw, self.pitch);
            let block = self.clip_ray(origin, direction, self.reach());
            if block_phase && (hit.is_none() || block.as_ref().map(|b| b.pos) != hit.map(|b| b.pos))
            {
                return false;
            }
            let target = self.mod_entity_target();
            let Some(p) = self.player.as_ref() else {
                return false;
            };
            let actor = ServerPlayer {
                id: self
                    .lan_acting_uuid
                    .map(uuid_string)
                    .unwrap_or_else(|| self.player_uuid_string()),
                name: self
                    .lan_acting_name
                    .clone()
                    .unwrap_or_else(|| crate::game::chat::PLAYER_NAME.into()),
                x: p.position.x,
                y: p.position.y,
                z: p.position.z,
                dimension: self.dim.id().into(),
                game_mode: match self.mode {
                    Mode::Survival => GameMode::Survival,
                    Mode::Creative => GameMode::Creative,
                    Mode::Adventure => GameMode::Adventure,
                    Mode::Spectator => GameMode::Spectator,
                },
                health: p.health.health(),
            };
            let original = self.held().cloned();
            let hand = match self.hand {
                super::super::Hand::Main => InteractionHand::Main,
                super::super::Hand::Off => InteractionHand::Off,
            };
            let item = ItemUse {
                item_id: original
                    .as_ref()
                    .map_or_else(|| "minecraft:air".into(), |s| s.item.to_string()),
                count: original.as_ref().map_or(0, |s| s.count),
                hand,
                target,
                block: block.as_ref().map(|b| InteractionPos {
                    x: b.pos[0],
                    y: b.pos[1],
                    z: b.pos[2],
                }),
            };
            let event = if block_phase {
                block.as_ref().map(|b| BlockUse {
                    state: self.state_string(self.world.block(b.pos[0], b.pos[1], b.pos[2])),
                    pos: InteractionPos {
                        x: b.pos[0],
                        y: b.pos[1],
                        z: b.pos[2],
                    },
                    hand,
                })
            } else {
                None
            };
            let mut players = self.mods.players.clone();
            players.retain(|p| p.id != actor.id);
            players.push(actor.clone());
            let snapshot = self.mod_snapshot_with(players);
            self.mods.mods.set_snapshot(snapshot);
            self.lend_world();
            let result = self.mods.mods.interact(&actor, &item, event.as_ref());
            self.return_world();
            self.show_mod_output();
            self.apply_mod_block_writes();
            match result {
                Ok(None) => false,
                Ok(Some(result)) => {
                    let valid = result.consume == 0
                        || match (&original, self.held()) {
                            (Some(before), Some(now)) => {
                                before == now && result.consume <= now.count
                            }
                            (None, None) => result.consume == 0,
                            _ => false,
                        };
                    self.mod_interaction_handled = true;
                    if !valid {
                        self.mod_interaction_feedback(
                            "The held stack changed during the interaction",
                            true,
                        );
                        return true;
                    }
                    if result.consume > 0 {
                        let cell = self.held_mut();
                        if let Some(stack) = cell {
                            stack.count -= result.consume;
                            if stack.count == 0 {
                                *cell = None;
                            }
                        }
                    }
                    if let Some(text) = result.feedback {
                        self.mod_interaction_feedback(&text, false);
                    }
                    self.swing_arm();
                    true
                }
                Err(error) => {
                    self.mod_interaction_handled = true;
                    self.mod_interaction_feedback(&error, true);
                    true
                }
            }
        }
        fn mod_interaction_feedback(&mut self, line: &str, error: bool) {
            let mut line = Text::literal(line.chars().take(1024).collect::<String>());
            if error {
                line = line.with_color(text::RED);
            }
            if self.lan_acting.is_some() {
                self.guest_packets.push(C::SystemChat(play::SystemChat {
                    content: component_nbt(&line.component()),
                    overlay: false,
                }));
            } else {
                self.system_message(line);
            }
        }

        /// The players on this game's server: the window's own player
        /// in-frame, then every connection's. `host` is the hub when it is
        /// out of `self.lan`.
        pub(in crate::game) fn server_players(&self, host: Option<&LanHost>) -> Vec<ServerPlayer> {
            let mut out = Vec::new();
            if self.options.execution_role.is_client()
                && self.remote.is_none()
                && let Some(p) = &self.player
            {
                out.push(ServerPlayer {
                    id: self.player_uuid_string(),
                    name: crate::game::chat::PLAYER_NAME.to_owned(),
                    x: p.position.x,
                    y: p.position.y,
                    z: p.position.z,
                    dimension: self.dim.id().to_owned(),
                    game_mode: match self.mode {
                        Mode::Survival => GameMode::Survival,
                        Mode::Creative => GameMode::Creative,
                        Mode::Adventure => GameMode::Adventure,
                        Mode::Spectator => GameMode::Spectator,
                    },
                    health: p.health.health(),
                });
            }
            let guests = host
                .or(self.lan.as_ref())
                .into_iter()
                .flat_map(|h| &h.hub.guests);
            out.extend(guests.filter(|g| g.placed).map(|g| ServerPlayer {
                id: uuid_string(g.uuid),
                name: g.name.clone(),
                x: g.position[0],
                y: g.position[1],
                z: g.position[2],
                dimension: g.dimension.clone(),
                game_mode: mode(g.game_mode),
                health: g.health,
            }));
            out
        }

        /// Whether what players do now reaches server mods: one is running
        /// and this game simulates the world.
        fn hears_players(&self) -> bool {
            self.mods.server && self.remote.is_none()
        }

        /// The tick's end: joins and leaves, the tick's events, the tick hook.
        pub(in crate::game) fn tick_server_mods(&mut self) {
            if !self.options.execution_role.is_client() {
                // A server has no frames to check its dev mods in.
                self.reload_dev_mods();
            }
            self.send_server_mod_commands();
            if !self.mods.server {
                self.return_mod_entities();
                return;
            }
            let players = self.server_players(None);
            let snapshot = self.mod_snapshot_with(players.clone());
            self.mods.mods.set_snapshot(snapshot);
            self.lend_world();
            let before = std::mem::take(&mut self.mods.players);
            for p in before
                .iter()
                .filter(|p| !players.iter().any(|q| q.id == p.id))
            {
                self.mods.mods.player_left(p);
            }
            for p in players
                .iter()
                .filter(|p| !before.iter().any(|q| q.id == p.id))
            {
                self.mods.mods.player_joined(p);
            }
            for event in std::mem::take(&mut self.mods.events) {
                let who = |w: &Who| {
                    let id = match w {
                        Who::Local => self.player_uuid_string(),
                        Who::Uuid(u) => uuid_string(*u),
                        Who::Conn(c) => {
                            let guests = self.lan.iter().flat_map(|h| &h.hub.guests);
                            let g = guests.into_iter().find(|g| g.conn == *c);
                            g.map_or_else(String::new, |g| uuid_string(g.uuid))
                        }
                    };
                    players.iter().chain(&before).find(|p| p.id == id).cloned()
                };
                match event {
                    ModEvent::Chat(w, text) => {
                        if let Some(p) = who(&w) {
                            self.mods.mods.player_chat(&p, &text);
                        }
                    }
                    ModEvent::Block {
                        who: w,
                        pos,
                        state,
                        placed,
                    } => {
                        let Some(p) = who(&w) else { continue };
                        let pos = BlockPos {
                            x: pos[0],
                            y: pos[1],
                            z: pos[2],
                        };
                        if placed {
                            self.mods.mods.block_placed(&p, pos, &state);
                        } else {
                            self.mods.mods.block_broken(&p, pos, &state);
                        }
                    }
                }
            }
            self.mods.mods.server_tick(self.ticks);
            self.return_world();
            self.mods.players = players;
            self.show_mod_output();
            self.apply_mod_block_writes();
        }

        /// Players' command trees list the running server mods' commands
        /// after vanilla's, whenever they change (or a hub opens).
        fn send_server_mod_commands(&mut self) {
            let key = self.mods.mods.generation();
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            if host.hub.mod_commands_key() == Some(key) {
                return;
            }
            let tree = command_tree(self.mods.mods.server_commands());
            host.hub.set_mod_commands(key, tree);
        }

        fn lend_mod_entities(&mut self) {
            if !self.mods.mods.wants_entities()
                || self.remote.is_some()
                || (self.options.execution_role.is_client() && self.player.is_none())
            {
                return;
            }
            let id = match self.mod_entity_world {
                Some(id) => id,
                None => match horizon_mods::entities::WorldId::new() {
                    Ok(id) => {
                        self.mod_entity_world = Some(id);
                        id
                    }
                    Err(e) => {
                        log::error!("Horizon: {e}");
                        return;
                    }
                },
            };
            let dim = self.dim;
            let mut levels = vec![self.mod_entities.loaded(
                dim.id(),
                dim.min_y(),
                dim.max_y() + 1,
                &self.world.chunks,
            )];
            for (dim, entities, chunks) in self.levels.entity_levels() {
                levels.push(entities.loaded(dim.id(), dim.min_y(), dim.max_y() + 1, chunks));
            }
            self.mods
                .mods
                .lend_entities(horizon_mods::entities::World::new(
                    id,
                    self.next_entity,
                    levels,
                ));
        }
        pub(in crate::game) fn return_mod_entities(&mut self) {
            if let Some(world) = self.mods.mods.take_entities() {
                self.next_entity = world.last_id();
            }
            self.mod_entities.prune();
            for (_, entities, _) in self.levels.entity_levels() {
                entities.prune();
            }
            self.trace_mod_entities();
        }

        /// Lends the levels' chunks to capability `world` for the calls up to
        /// [`Game::return_world`], when a running server mod has it.
        fn lend_world(&mut self) {
            self.lend_mod_inventory(None);
            self.sync_mod_messages();
            self.lend_mod_entities();
            if self.mods.mods.wants_world_storage()
                && self.remote.is_none()
                && !self.world_root.as_os_str().is_empty()
            {
                if self.mod_entity_world.is_none() {
                    self.mod_entity_world = horizon_mods::entities::WorldId::new().ok();
                }
                let admission = self.world_root.persistence().cloned().map(|owner| {
                    std::sync::Arc::new(SnapshotAdmission(owner))
                        as std::sync::Arc<dyn horizon_mods::WorldStorageAdmission>
                });
                self.mods
                    .mods
                    .set_world_storage(self.mod_entity_world.zip(admission).map(
                        |(id, admission)| {
                            (
                                id.get(),
                                self.world_root.as_ref(),
                                !self.mods.storage_readonly,
                                admission,
                            )
                        },
                    ));
            } else {
                self.mods.mods.set_world_storage(None);
            }
            if !self.mods.world && !self.mods.mods.wants_entities() {
                return;
            }
            let mut heights = vec![(self.dim.id(), self.dim.min_y(), self.dim.max_y())];
            let mut dims = vec![(self.dim.id(), std::mem::take(&mut self.world.chunks))];
            for (dim, chunks) in self.levels.parked_chunks() {
                dims.push((dim.id(), std::mem::take(chunks)));
                heights.push((dim.id(), dim.min_y(), dim.max_y()));
            }
            let registry = self.pack.assets.registry;
            let biomes = self.res.biomes().clone();
            self.mods.mods.lend_world(Box::new(LentWorld {
                registry,
                biomes,
                dims,
                resources: self.res.clone(),
                heights,
                properties: self.pack.assets.properties.clone(),
                clock: (self.world.game_time, self.world.day_time),
                weather: self.world.weather,
            }));
        }

        /// Puts back the chunks [`Game::lend_world`] lent.
        fn return_world(&mut self) {
            self.return_mod_inventory(None);
            self.return_mod_entities();
            let Some(view) = self.mods.mods.take_world() else {
                return;
            };
            let view: Box<dyn std::any::Any> = view;
            let Ok(lent) = view.downcast::<LentWorld>() else {
                return;
            };
            let mut dims = lent.dims;
            if let Some(i) = dims.iter().position(|(d, _)| *d == self.dim.id()) {
                self.world.chunks = dims.swap_remove(i).1;
            }
            for (dim, chunks) in self.levels.parked_chunks() {
                if let Some(i) = dims.iter().position(|(d, _)| *d == dim.id()) {
                    *chunks = dims.swap_remove(i).1;
                }
            }
        }

        /// Makes the blocks server mods set in the level in place, in order,
        /// as `/setblock` does. Those of other dimensions wait for their level.
        pub(in crate::game) fn apply_mod_block_writes(&mut self) {
            if self.mods.block_writes.is_empty() {
                return;
            }
            let dim = self.dim.id();
            let (now, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.mods.block_writes)
                .into_iter()
                .partition(|(d, _)| d == dim);
            self.mods.block_writes = later;
            let registry = self.pack.assets.registry;
            for (_, b) in now {
                let state = match crate::game::render_control::parse_state(registry, &b.state) {
                    Ok(state) => state,
                    Err(e) => {
                        log::warn!("Horizon: a server mod set {}: {e}", b.state);
                        continue;
                    }
                };
                if let Err(e) = self.place_state([b.x, b.y, b.z], state, false) {
                    log::debug!(
                        "Horizon: a server mod's block at {} {} {}: {e:?}",
                        b.x,
                        b.y,
                        b.z
                    );
                }
            }
        }

        /// A player broke (or placed) `state` at `pos`: the player acting now.
        pub(in crate::game) fn mod_block_event(
            &mut self,
            pos: [i32; 3],
            state: BlockStateId,
            placed: bool,
        ) {
            if !self.hears_players() {
                return;
            }
            let who = self.lan_acting.map_or(Who::Local, Who::Conn);
            let state = self.state_string(state);
            self.mods.events.push(ModEvent::Block {
                who,
                pos,
                state,
                placed,
            });
        }

        /// A chat message a player sent.
        pub(in crate::game) fn mod_player_chat(&mut self, who: Who, text: &str) {
            if self.hears_players() {
                self.mods.events.push(ModEvent::Chat(who, text.to_owned()));
            }
        }

        /// `/line` typed by the window's own player in-frame: true when a
        /// server mod owns it.
        pub(in crate::game) fn local_server_mod_command(&mut self, line: &str) -> bool {
            if !self.hears_players() {
                return false;
            }
            let players = self.server_players(None);
            let Some(me) = players.first().cloned() else {
                return false;
            };
            let snapshot = self.mod_snapshot_with(players);
            self.mods.mods.set_snapshot(snapshot);
            self.lend_world();
            let owned = self.mods.mods.server_command(&me, line);
            self.return_world();
            self.show_mod_output();
            self.apply_mod_block_writes();
            owned
        }

        /// `/line` typed by a connection's player: true when a server mod
        /// owns it, whatever the player's permission level.
        pub(in crate::game) fn guest_server_mod_command(
            &mut self,
            host: &mut LanHost,
            conn: u32,
            line: &str,
        ) -> bool {
            if !self.hears_players() {
                return false;
            }
            let Some(g) = host.hub.guest(conn) else {
                return false;
            };
            let id = uuid_string(g.uuid);
            let players = self.server_players(Some(host));
            let Some(me) = players.iter().find(|p| p.id == id).cloned() else {
                return false;
            };
            let snapshot = self.mod_snapshot_with(players);
            self.mods.mods.set_snapshot(snapshot);
            self.lend_mod_inventory(Some(host));
            self.lend_world();
            let owned = self.mods.mods.server_command(&me, line);
            self.return_mod_inventory(Some(host));
            self.return_world();
            if !owned {
                return false;
            }
            self.route_mod_output(Some(host));
            self.apply_mod_block_writes();
            true
        }

        /// Sends a server mod's line to the player with this id: the
        /// window's own chat for the local player, a system message for a
        /// connection's. `host` is the hub when it is out of `self.lan`.
        pub(in crate::game) fn tell_server_player(
            &mut self,
            host: Option<&mut LanHost>,
            id: &str,
            line: Text,
        ) {
            let local = self.options.execution_role.is_client()
                && self.remote.is_none()
                && self.player.is_some();
            if local && self.player_uuid_string() == id {
                self.add_mod_chat_line(line);
                return;
            }
            let Some(host) = host.or(self.lan.as_mut()) else {
                return;
            };
            let to = host
                .hub
                .guests
                .iter()
                .find(|g| g.placed && uuid_string(g.uuid) == id);
            if let Some(conn) = to.map(|g| g.conn) {
                host.hub.send_to(
                    conn,
                    &C::SystemChat(play::SystemChat {
                        content: component_nbt(&line.component()),
                        overlay: false,
                    }),
                );
            }
        }

        /// Routes a server mod's output: `Tell` and `Reply` to their player,
        /// `Broadcast` to everyone and the log.
        pub(in crate::game) fn route_server_output(
            &mut self,
            mut host: Option<&mut LanHost>,
            output: Output,
        ) {
            let named = |name: &str, text: String| {
                let mut line = Text::literal(format!("[{name}] ")).with_color(text::GRAY);
                line.push(Text::literal(text));
                line
            };
            match output {
                Output::Tell { name, player, text } => {
                    self.tell_server_player(host, &player, named(&name, text));
                }
                Output::Reply {
                    player,
                    text,
                    error,
                } => {
                    let line = Text::literal(text);
                    let line = if error {
                        line.with_color(text::RED)
                    } else {
                        line
                    };
                    self.tell_server_player(host, &player, line);
                }
                Output::Broadcast { name, text } => {
                    let plain = format!("[{name}] {text}");
                    if self.options.dedicated.is_some() {
                        super::super::dedicated::log("INFO", &plain);
                    } else {
                        log::info!("{plain}");
                    }
                    for p in self.server_players(host.as_deref()) {
                        let line = named(&name, text.clone());
                        self.tell_server_player(host.as_deref_mut(), &p.id, line);
                    }
                }
                Output::SetBlocks {
                    name,
                    dimension,
                    blocks,
                } => {
                    let queued = &mut self.mods.block_writes;
                    queued.extend(blocks.into_iter().map(|b| (dimension.clone(), b)));
                    if queued.len() > MAX_QUEUED_WRITES {
                        let over = queued.len() - MAX_QUEUED_WRITES;
                        queued.drain(..over);
                        log::warn!(
                            "Horizon: mod {name} set more blocks than wait for their level: {over} dropped"
                        );
                    }
                }
                _ => {}
            }
        }
    }
}
