//! Beds' `occupied` (`BedBlock.OCCUPIED`): the head of a bed someone sleeps
//! in, which the foot copies (`updateShape`). `LivingEntity.startSleeping`
//! sets it and `stopSleeping` clears it. A villager's come with its mob
//! events (`MobEvent::BedOccupied`); a player's bed is set when it lies
//! down (`try_sleep`) and freed in the level's tick once nobody sleeps in it
//! any more, whichever way they got up: waking, the Leave Bed button, death,
//! a broken bed, or leaving the game.
use super::{Dim, Game, Instant, Mapping, Screen, facing_offset, respawn_anchor, tick_sleep_timer};
use horizon_client::Result;
use horizon_client::bed_geometry::{SleepOn, pos_to_bed, shape_top, slept_on};
use horizon_core::geometry::Vec3;
use horizon_world::registry::BlockStateId;

impl Game<'_> {
    /// `setBlockAndUpdate(head, state.setValue(OCCUPIED, occupied))`, when
    /// the block at `head` is still a bed and not already so.
    pub(super) fn set_bed_occupied(&mut self, head: [i32; 3], occupied: bool) {
        let registry = &self.pack.assets.registry;
        let Some(mut state) = registry
            .state(self.world.block(head[0], head[1], head[2]))
            .cloned()
        else {
            return;
        };
        let value = if occupied { "true" } else { "false" };
        if state.properties.get("occupied").is_none_or(|v| v == value) {
            return;
        }
        state.properties.insert("occupied".into(), value.into());
        let Some(id) = registry.id(&state) else {
            return;
        };
        if let Err(e) = self.level_set(head, id) {
            log::warn!("could not mark the bed at {head:?} occupied={value}: {e}");
        }
    }

    /// `LivingEntity.startSleeping` for the player in place (the host, or
    /// a guest during its turn): the bed it lies in is occupied.
    pub(super) fn occupy_bed(&mut self, head: [i32; 3]) {
        self.set_bed_occupied(head, true);
        if !self.player_beds.contains(&(self.dim, head)) {
            self.player_beds.push((self.dim, head));
        }
    }

    /// Whether a player in the level in place still sleeps in `head`.
    fn player_in_bed(&self, head: [i32; 3]) -> bool {
        self.sleeping == Some(head)
            || self.lan.as_ref().is_some_and(|h| {
                h.hub
                    .guests
                    .iter()
                    .any(|g| g.bed == Some(head) && g.dimension == self.dim.id())
            })
    }

    /// `LivingEntity.stopSleeping` for the players who got up: the beds of
    /// this level nobody sleeps in any more are free again. Runs in each
    /// level's tick, after the players' packets and sleep ticks, never inside
    /// a guest's turn (where the hub is taken out of `Game`).
    pub(super) fn free_left_beds(&mut self) {
        let dim = self.dim;
        let left: Vec<[i32; 3]> = self
            .player_beds
            .iter()
            .filter(|(d, head)| *d == dim && !self.player_in_bed(*head))
            .map(|(_, head)| *head)
            .collect();
        for head in left {
            self.player_beds.retain(|b| *b != (dim, head));
            self.set_bed_occupied(head, false);
            // AbstractBedBlock.onStopSleeping: the Overworld's straw_bed_rule
            // has destroy_on_leave, so a straw bed breaks when its sleeper gets up.
            let bed = self.world.block(head[0], head[1], head[2]);
            let straw = &*self.res.facts().get(bed).name == "minecraft:straw_bed";
            if straw
                && self.dimension_type_of(dim).straw_bed_rule.destroy_on_leave
                && let Err(e) = self.destroy_straw_bed(head)
            {
                log::warn!("could not break the straw bed at {head:?}: {e}");
            }
        }
    }

    /// The levels with a bed a player marked occupied: they tick until the
    /// bed is freed.
    pub(super) fn player_bed_levels(&self) -> impl Iterator<Item = Dim> + '_ {
        self.player_beds.iter().map(|(d, _)| *d)
    }

    /// `PlayerList.removeAll` before the server's last save: every player
    /// leaves (`ServerPlayer.disconnect` stops its sleep), so no bed is
    /// saved occupied by a player who is not in it when the world opens
    /// again. Each bed is freed in its own level.
    pub(super) fn free_player_beds(&mut self) -> Result<()> {
        if self.player_beds.is_empty() {
            return Ok(());
        }
        let back = self.dim;
        for (dim, head) in std::mem::take(&mut self.player_beds) {
            self.enter_level(None, dim)?;
            // A client holds no other levels (`enter_level`).
            if self.dim == dim {
                self.set_bed_occupied(head, false);
            }
        }
        self.enter_level(None, back)
    }
}

impl Game<'_> {
    /// The bed's `part`: head or foot.
    fn bed_part(&self, state: BlockStateId) -> Option<String> {
        self.pack
            .assets
            .registry
            .state(state)
            .and_then(|s| s.properties.get("part").cloned())
    }
    /// `LivingEntity.startSleeping`'s height: the top of the shape the player
    /// lies on, measured from the head's own y - a straw bed's head from its
    /// foot's - or `None` when there is no such shape, in which case the
    /// player does not lie down at all.
    pub(super) fn sleep_height(&self, head: [i32; 3]) -> Option<f64> {
        let state = self.world.block(head[0], head[1], head[2]);
        let name = self.res.facts().get(state).name.to_owned();
        let part = self.bed_part(state);
        let step = self.bed_forward(state);
        let foot_at = [head[0] - step[0], head[1], head[2] - step[2]];
        let foot_state = self.world.block(foot_at[0], foot_at[1], foot_at[2]);
        let foot_name = self.res.facts().get(foot_state).name.to_owned();
        let foot_part = self.bed_part(foot_state);
        let on = slept_on(
            &name,
            part.as_deref(),
            Some((&foot_name, foot_part.as_deref())),
        )?;
        let at = match on {
            SleepOn::Head => head,
            SleepOn::Foot => foot_at,
        };
        shape_top(&self.outline_boxes(at), f64::from(at[1]))
    }
    /// Where the sleeper lies (`setPosToBed`), when the bed gives it a shape.
    pub(super) fn sleeping_position(&self, head: [i32; 3]) -> Option<Vec3> {
        self.sleep_height(head)
            .map(|height| pos_to_bed(head, height))
    }
}

impl Game<'_> {
    /// A translated message for the player the game acts for: `overlay` is
    /// `sendOverlayMessage` (the action bar), else `sendSystemMessage` (chat).
    /// A guest gets a packet, so its own client translates it.
    pub(super) fn tell_sleeper(&mut self, key: &str, fallback: &str, overlay: bool) {
        if self.lan_acting.is_some() {
            self.guest_packets
                .push(horizon_protocol::packets::play::Clientbound::SystemChat(
                    horizon_protocol::packets::play::SystemChat {
                        content: horizon_protocol::text::translatable(key, &[]),
                        overlay,
                    },
                ));
        } else if overlay {
            self.message = Some((self.translate(key, fallback), Instant::now()));
        } else {
            self.system_message(horizon_client::text::Text::translate(key, &[]));
        }
    }
    /// ServerPlayer.startSleepInBed: in reach, not obstructed, night or thunder,
    /// and (outside creative) no monsters within 8 blocks; the bed becomes the
    /// respawn point either way.
    pub(super) fn try_sleep(&mut self, pos: [i32; 3]) {
        let Some((position, yaw)) = self.player.as_ref().map(|p| (p.position, p.yaw)) else {
            return;
        };
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let props = self
            .pack
            .assets
            .registry
            .state(state)
            .map(|s| s.properties.clone())
            .unwrap_or_default();
        // The head and the foot: the block faces from the foot to the head.
        let d = facing_offset(props.get("facing").map_or("north", String::as_str));
        let (head, foot) = if props.get("part").map(String::as_str) == Some("foot") {
            ([pos[0] + d[0], pos[1], pos[2] + d[2]], pos)
        } else {
            (pos, [pos[0] - d[0], pos[1], pos[2] - d[2]])
        };
        // AbstractBedBlock.useWithoutItem: in an occupied bed a villager
        // asleep there is woken (kickVillagerOutOfBed), else someone else
        // sleeps in it; the player stays up either way.
        let head_state = self.world.block(head[0], head[1], head[2]);
        let occupied = self
            .pack
            .assets
            .registry
            .state(head_state)
            .is_some_and(|s| s.properties.get("occupied").is_some_and(|v| v == "true"));
        if occupied {
            if !self.kick_villager_out_of_bed(head) {
                self.tell_sleeper("block.minecraft.bed.occupied", "This bed is occupied", true);
            }
            return;
        }
        // ServerPlayer.bedInRange / isReachableBedBlock: within 3 blocks
        // sideways and 2 vertically of the head or the foot.
        let reachable = |bed: [i32; 3]| {
            (position.x - f64::from(bed[0]) - 0.5).abs() <= 3.0
                && (position.y - f64::from(bed[1])).abs() <= 2.0
                && (position.z - f64::from(bed[2]) - 0.5).abs() <= 3.0
        };
        let mut problem: Option<&'static str> = if !reachable(head) && !reachable(foot) {
            Some("too_far_away")
        } else {
            None
        };
        // ServerPlayer.bedBlocked: Player.freeAt (not suffocating) for the
        // blocks above the head and the foot, as `in_wall` reads suffocating.
        let blocked = problem.is_none()
            && [head, foot].iter().any(|bed| {
                let pos = [bed[0], bed[1] + 1, bed[2]];
                let state = self.world.block(pos[0], pos[1], pos[2]);
                let mut scene = horizon_client::collisions::SceneCollisions {
                    world: &self.world,
                    binding: self.mob_suffocation().collisions(),
                    subject: None,
                };
                self.mob_suffocation()
                    .is_suffocating(&mut scene, state, pos)
                    .unwrap_or_else(|error| {
                        log::warn!("could not check bed obstruction at {pos:?}: {error}");
                        true
                    })
            });
        if blocked {
            problem = Some("obstructed");
        }
        // In reach (bedInRange), day or night: setRespawnPosition(
        // RespawnConfig(the head, the player's yaw), not forced). It replaces
        // an anchor or a spawnpoint; respawning checks the bed is still there
        // and finds where to stand up (findRespawnAndUseSpawnBlock).
        // Straw beds never set it (`straw_bed_rule`).
        if problem.is_none() && self.bed_sets_spawn(state) {
            let config = respawn_anchor::RespawnConfig::wrapped(self.dim, head, yaw, false);
            // ServerPlayer.setRespawnPosition: the chat message only when the
            // point moved (`isSamePosition`).
            let moved = self
                .respawn_config
                .is_none_or(|c| (c.dim, c.pos) != (config.dim, config.pos));
            let mut cancel = false;
            if moved
                && let (Some(player), Some(name)) =
                    (self.lan_acting_uuid, self.lan_acting_name.clone())
            {
                self.native_player_spawn_point_set(
                    player,
                    &name,
                    self.dim.id(),
                    head,
                    "bed",
                    &mut cancel,
                );
            }
            if !cancel {
                self.respawn_config = Some(config);
                if moved {
                    self.tell_sleeper("block.minecraft.set_spawn", "Respawn point set", false);
                }
                // The fallback becomes the world spawn: a guest's is none of its
                // own, so it follows `setworldspawn`.
                self.spawn = if self.lan_acting.is_some() {
                    None
                } else {
                    self.level
                        .world_spawn
                        .map(|[x, y, z]| {
                            Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5)
                        })
                        .or(self.spawn)
                };
            }
        }
        if problem.is_none()
            && !self
                .dimension_type()
                .bed_rule
                .can_sleep
                .test(self.dark_outside())
        {
            problem = Some("not_possible_now");
        }
        // Creative players skip the monster check (`!this.isCreative()`).
        if problem.is_none()
            && !self.creative
            && self.mobs.iter().any(|m| {
                // `Monster.class` entities that `isPreventingPlayerRest`: a
                // zombified piglin only while it is angry.
                horizon_client::mobs::ai::is_monster(m.species())
                    && (m.species() != horizon_client::mobs::Species::ZombifiedPiglin
                        || (m.extra.anger > 0 && m.ai.target.is_some()))
                    && m.alive()
                    && m.bounds().intersects(horizon_core::geometry::Aabb::new(
                        Vec3::new(
                            f64::from(head[0]) + 0.5 - 8.0,
                            f64::from(head[1]) - 5.0,
                            f64::from(head[2]) + 0.5 - 8.0,
                        ),
                        Vec3::new(
                            f64::from(head[0]) + 0.5 + 8.0,
                            f64::from(head[1]) + 5.0,
                            f64::from(head[2]) + 0.5 + 8.0,
                        ),
                    ))
            })
        {
            problem = Some("not_safe");
        }
        // NeoForge's CanPlayerSleepEvent: a mod may clear or replace the
        // problem vanilla found, or add one.
        let mut problem = problem.map(str::to_owned);
        if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone()) {
            self.native_can_player_sleep(player, &name, head, &mut problem);
        }
        if let Some(problem) = problem {
            match problem.as_str() {
                "too_far_away" => self.tell_sleeper(
                    "block.minecraft.bed.too_far_away",
                    "You may not rest now; the bed is too far away",
                    true,
                ),
                "obstructed" => self.tell_sleeper(
                    "block.minecraft.bed.obstructed",
                    "This bed is obstructed",
                    true,
                ),
                "not_safe" => self.tell_sleeper(
                    "block.minecraft.bed.not_safe",
                    "You may not rest now; there are monsters nearby",
                    true,
                ),
                "not_possible_now" => self.tell_sleeper(
                    "block.minecraft.bed.no_sleep",
                    "You can sleep only at night or during thunderstorms",
                    true,
                ),
                // `other_problem` and any name a mod invents: refused without a message.
                _ => {}
            }
            return;
        }
        // ServerPlayer.startSleepInBed: a percentage above 100 cannot skip the night.
        if self.ui.rules.int("players_sleeping_percentage") > 100 {
            self.tell_sleeper(
                "sleep.not_possible",
                "No amount of rest can pass this night",
                true,
            );
        }
        if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone()) {
            let mut cancel = false;
            self.native_player_bed_enter(player, &name, head, &mut cancel);
            if cancel {
                return;
            }
        }
        // LivingEntity.startSleeping: the shape the bed gives the player to
        // lie on (`getSleepHeight`). A straw bed's head answers with its
        // foot's shape, and with no foot of its own there is none, so the
        // player stays up as `startSleeping` returning false leaves them.
        let Some(at) = self.sleeping_position(head) else {
            return;
        };
        // LivingEntity.startSleeping: stopRiding once the bed is valid.
        self.stop_riding();
        self.stop_mining();
        self.left = false;
        self.right = false;
        self.using = None;
        self.sleeping = Some(head);
        self.sleep_timer = 0;
        // LivingEntity.startSleeping and ServerPlayer.startSleeping: setPosToBed
        // and connection.teleport.
        if let Some(p) = self.player.as_mut() {
            p.position = at;
            p.velocity = Vec3::ZERO;
            p.pitch = 0.0;
            p.teleported = true;
            let _ = p.set_sleeping(true);
        }
        self.pitch = 0.0;
        if self.lan_acting.is_none() {
            self.previous_position = at;
            self.snap_player_model_pitch();
        }
        self.occupy_bed(head);
        let straw = &*self.res.facts().get(state).name == "minecraft:straw_bed";
        self.award_custom_stat(
            if straw {
                "sleep_in_straw_bed"
            } else {
                "sleep_in_bed"
            },
            1,
        );
        if self.bed_sets_spawn(state) {
            self.trigger_player("minecraft:slept_in_bed");
        }
        self.stats.reset("custom", "minecraft:time_since_rest");
    }
    pub(super) fn wake_up(&mut self) {
        if self.remote.is_some() {
            // InBedChatScreen.sendWakeUp: the server wakes the player.
            self.remote_stop_sleeping();
            return;
        }
        let Some(head) = self.sleeping.take() else {
            return;
        };
        self.sleep_timer = 100;
        let stand = self.bed_wake_pose(head, self.yaw);
        if let Some(p) = self.player.as_mut() {
            if let Some((at, yaw)) = stand {
                p.position = at;
                p.velocity = Vec3::ZERO;
                p.yaw = yaw;
                p.pitch = 0.0;
            }
            let _ = p.set_sleeping(false);
        }
        if let Some((at, yaw)) = stand {
            self.previous_position = at;
            self.yaw = yaw;
            self.pitch = 0.0;
            self.snap_player_model_pitch();
        }
        if let (Some(player), Some(name)) = (self.lan_acting_uuid, self.lan_acting_name.clone()) {
            self.native_player_bed_leave(player, &name, head);
        }
    }
    pub(super) fn tick_sleep(&mut self) {
        self.sleep_timer = tick_sleep_timer(self.sleep_timer, self.sleeping.is_some());
        if let Some(head) = self.sleeping {
            // LivingEntity.tick's checkBedExists: a sleeper whose bed is
            // gone gets up.
            let bed = self.world.block(head[0], head[1], head[2]);
            let bed_gone = !self.res.facts().get(bed).name.ends_with("_bed");
            // Vanilla has no Shift wake-up: the chat screen's capitals must
            // not get the sleeper up.
            if (self.screen != Screen::Chat && self.keys.held(Mapping::Sneak))
                || bed_gone
                || !(self.dim == Dim::Overworld && self.sky_darken() >= 4)
                || self
                    .player
                    .as_ref()
                    .is_none_or(|p| p.health.dead_or_dying() || !p.sleeping)
            {
                self.wake_up();
                return;
            }
            // setPosToBed: the bed's top (`getSleepHeight`) and 0.125 above
            // it. A sleeper whose bed no longer gives a shape to lie on stays
            // where it is: vanilla only ever places a sleeper when it starts.
            let at = self.sleeping_position(head);
            if let (Some(p), Some(at)) = (self.player.as_mut(), at) {
                p.position = at;
                p.velocity = Vec3::ZERO;
                p.pitch = 0.0;
            }
            // LivingEntity.tick: a sleeper's xRot is 0.
            self.pitch = 0.0;
            self.snap_player_model_pitch();
        }
        // ServerLevel.tick: once enough of the players have slept 100 ticks,
        // skip to morning if `advance_time` is on, wake everyone, and
        // resetWeatherCycle if `advance_weather` is on and it is raining.
        if self.enough_slept() {
            if self.ui.rules.bool("advance_time") {
                let t = self.world.day_time;
                let mut target = t + 24000 - t.rem_euclid(24000);
                // SleepFinishedTimeEvent: the mods may move the wake-up time,
                // never before now (`setTimeAddition` refuses a lower one).
                let mut asked = target;
                self.native_sleep_finished_time(self.dim.id(), t, &mut asked);
                if asked >= t {
                    target = asked;
                }
                let mut cancel = false;
                self.native_time_skip(self.dim.id(), t, target, "sleep", &mut cancel);
                if !cancel {
                    self.world.day_time = target;
                }
            }
            // Level.isRaining: the rain level is above 0.2.
            if self.ui.rules.bool("advance_weather") && self.weather_levels(1.0).0 > 0.2 {
                self.world.weather.raining = false;
                self.world.weather.thundering = false;
                self.world.weather.rain_time = 0;
                self.world.weather.thunder_time = 0;
            }
            if self.sleeping.is_some() {
                self.wake_up();
            }
            self.wake_guests();
            // SleepStatus.removeAllSleepers: waking everyone is not announced.
            self.sleep_seen.1 = 0;
        }
    }
}
