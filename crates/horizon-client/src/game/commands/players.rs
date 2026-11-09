//! Commands acting on the other players of a hosted world or a server
//! (EntitySelector.findPlayers and EntityArgument's player targets). The
//! player in place (the host's, or the guest whose turn it is) is
//! `Target::Player`; every other guest is `Target::Guest`, found by name,
//! UUID or selector, and what a command does to them runs with their state
//! swapped in (`for_player`), as their own turn, what it tells them going to
//! their chat.
use super::{Game, Target};
use horizon_client::{command::args::Selector, lan::guest::Guest, text::Text};
use horizon_protocol::packets::play::{self, Clientbound as C};

impl Target {
    /// A player, in place or not (`instanceof ServerPlayer`).
    pub(in crate::game) fn is_player(self) -> bool {
        matches!(self, Target::Player | Target::Guest(_))
    }
}

/// GameType.getName for a guest's game mode id.
fn mode_name(id: i32) -> &'static str {
    match id {
        1 => "creative",
        2 => "adventure",
        3 => "spectator",
        _ => "survival",
    }
}

impl Game<'_> {
    /// The guest `conn`, while the hub is in `self.lan`.
    pub(in crate::game) fn command_guest(&self, conn: u32) -> Option<&Guest> {
        self.lan.as_ref()?.hub.guest(conn)
    }

    /// `PlayerList.getPlayers` but the player in place: the guests who
    /// joined the world.
    fn command_players(&self) -> impl Iterator<Item = &Guest> {
        let acting = self.lan_acting;
        self.lan
            .iter()
            .flat_map(|h| h.hub.guests.iter())
            .filter(move |g| g.placed && Some(g.conn) != acting)
    }

    /// The entity the command runs as (`CommandSourceStack.getEntity`): a
    /// command typed by the player in place runs as them, the server
    /// console's as nobody.
    pub(in crate::game) fn source_entity(&self) -> Option<Target> {
        match self.command_source {
            Some(s) => s.entity,
            None => self.player.is_some().then_some(Target::Player),
        }
    }

    /// `CommandSourceStack.getPlayerOrException` for the commands whose
    /// player argument is optional: the source's player (the player in
    /// place when it has none, as before).
    pub(in crate::game) fn implied_player(&self) -> Target {
        self.source_entity()
            .filter(|t| t.is_player())
            .unwrap_or(Target::Player)
    }

    /// `PlayerList.getPlayerByName`: names are not case sensitive. Horizon:
    /// on a server where a mod gave players nicknames
    /// (`native_set_nickname`), a name no player has finds the player going
    /// by it, without its `~` or with it (quoted), so `/msg` and `/tell`
    /// reach a player by the name chat shows. No nickname, no change.
    pub(super) fn player_named(&self, name: &str) -> Option<Target> {
        if self.player.is_some() && name.eq_ignore_ascii_case(self.acting_player_name()) {
            return Some(Target::Player);
        }
        self.command_players()
            .find(|g| g.name.eq_ignore_ascii_case(name))
            .map(|g| Target::Guest(g.conn))
            .or_else(|| self.player_nicknamed(name))
    }

    /// The player whose nickname is `name` (any case, a leading `~` either
    /// side ignored).
    fn player_nicknamed(&self, name: &str) -> Option<Target> {
        let bare = |s: &str| s.strip_prefix('~').unwrap_or(s).to_ascii_lowercase();
        let wanted = bare(name);
        let hub = &self.lan.as_ref()?.hub;
        let uuid = hub
            .nicknames
            .iter()
            .find(|(_, nick)| bare(nick) == wanted)?
            .0;
        self.target_by_uuid(*uuid)
    }

    /// `EntitySelector.findEntities` with a UUID: the player with it, else
    /// the entity.
    pub(in crate::game) fn target_by_uuid(&self, uuid: u128) -> Option<Target> {
        let own = self.lan_acting_uuid.or_else(|| {
            self.level.player_uuid.map(|[a, b, c, d]| {
                (u128::from(a as u32) << 96)
                    | (u128::from(b as u32) << 64)
                    | (u128::from(c as u32) << 32)
                    | u128::from(d as u32)
            })
        });
        if self.player.is_some() && own == Some(uuid) {
            return Some(Target::Player);
        }
        if let Some(g) = self.command_players().find(|g| g.uuid == uuid) {
            return Some(Target::Guest(g.conn));
        }
        let words = [96, 64, 32, 0].map(|shift| (uuid >> shift) as u32 as i32);
        self.target_with_uuid(words)
    }

    /// The guests a selector looks at, in join order (`PlayerList.getPlayers`),
    /// the one in place as `Target::Player`: all of them, or those in the
    /// source's level when it limits the search to a place
    /// (`isWorldLimited`); `@e` and `@n` skip the dead (`Entity::isAlive`).
    pub(super) fn player_candidates(&self, s: &Selector) -> Vec<Target> {
        let limited = s.distance.is_some()
            || s.pos.iter().any(Option::is_some)
            || s.delta.iter().any(Option::is_some);
        let alive = matches!(s.kind, 'e' | 'n');
        let here = self.dim.id();
        let in_place = self.lan_acting.filter(|_| self.player.is_some());
        self.lan
            .iter()
            .flat_map(|h| h.hub.guests.iter())
            .filter(|g| g.placed)
            .filter(|g| !limited || g.dimension == here || Some(g.conn) == in_place)
            .filter(|g| !alive || (!g.dead && g.health > 0.0))
            .map(|g| {
                if Some(g.conn) == in_place {
                    Target::Player
                } else {
                    Target::Guest(g.conn)
                }
            })
            .collect()
    }

    /// A guest's experience level and game mode, for `level=` and `gamemode=`.
    pub(super) fn guest_level_and_mode(&self, conn: u32) -> Option<(i32, &'static str)> {
        self.command_guest(conn)
            .map(|g| (g.xp_level, mode_name(g.game_mode)))
    }

    /// `Player.getDisplayName`: the name as its team shows it, suggesting a
    /// `/tell`, showing the player on hover, inserting the name.
    pub(in crate::game) fn player_text(&self, name: &str, uuid: u128) -> Text {
        let shown = match self.score.board.players_team(name) {
            Some(team) => team.formatted_name(Text::literal(name)),
            None => Text::literal(name),
        };
        shown.with_component_fields(vec![
            (
                "click_event".into(),
                serde_json::json!({"action": "suggest_command", "command": format!("/tell {name} ")}),
            ),
            (
                "hover_event".into(),
                serde_json::json!({"action": "show_entity", "id": "minecraft:player",
                    "uuid": horizon_protocol::crypto::uuid_string(uuid), "name": name}),
            ),
            ("insertion".into(), name.into()),
        ])
    }

    /// `Entity.getDisplayName`: name as team shows it, showing entity on hover, inserting its UUID.
    pub(in crate::game) fn mob_text(&self, m: &crate::mobs::Mob) -> Text {
        let team = self.mob_team_of(m);
        let uuid = self
            .persist
            .uuids
            .get(&m.id)
            .copied()
            .unwrap_or_else(crate::save::random_uuid);
        format_mob_text(m, team, uuid)
    }

    /// Run `f` for the player `t` (None for anything else): in place, or
    /// with the guest's state swapped in, in their level, as their own turn.
    /// What `f` tells the player (`system_message`) goes to their chat; the
    /// command's own feedback is the caller's to send.
    pub(in crate::game) fn for_player<T>(
        &mut self,
        t: Target,
        f: impl FnOnce(&mut Self) -> T,
    ) -> Option<T> {
        let conn = match t {
            Target::Player => return Some(f(self)),
            Target::Guest(conn) if self.lan_acting == Some(conn) => return Some(f(self)),
            Target::Guest(conn) => conn,
            _ => return None,
        };
        let mut host = self.lan.take()?;
        let back = self.dim;
        let theirs = host
            .hub
            .guest(conn)
            .map(|g| super::super::Dim::from_id(&g.dimension));
        if let Some(dim) = theirs.filter(|d| *d != back)
            && let Err(e) = self.enter_level(Some(&mut host), dim)
        {
            log::error!("could not open {}: {e}", dim.id());
        }
        let Some(swap) = self.enter_guest(&mut host, conn, back) else {
            if self.dim != back
                && let Err(e) = self.enter_level(Some(&mut host), back)
            {
                log::error!("could not reopen {}: {e}", back.id());
            }
            self.lan = Some(host);
            return None;
        };
        self.lan = Some(host);
        let source = self.chat_capture.replace(Vec::new());
        let out = f(self);
        let told = std::mem::replace(&mut self.chat_capture, source).unwrap_or_default();
        let mut host = self
            .lan
            .take()
            .expect("the hub is back after a player's turn");
        self.leave_guest(&mut host, swap);
        for line in told {
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content: horizon_client::lan::data::component_nbt(&line.component()),
                    overlay: false,
                }),
            );
        }
        if self.dim != back
            && let Err(e) = self.enter_level(Some(&mut host), back)
        {
            log::error!("could not reopen {}: {e}", back.id());
        }
        self.lan = Some(host);
        Some(out)
    }
}

/// `Entity.getDisplayName`: name as team shows it, showing entity on hover, inserting its UUID.
pub(in crate::game) fn format_mob_text(
    m: &crate::mobs::Mob,
    team: Option<&horizon_client::scoreboard::Team>,
    uuid: [i32; 4],
) -> Text {
    let name = m.command_name();
    let shown = match team {
        Some(team) => team.formatted_name(name.clone()),
        None => name.clone(),
    };
    let uuid_str = horizon_client_core::uuid::uuid_text(uuid);
    shown.with_component_fields(vec![
        (
            "hover_event".into(),
            serde_json::json!({
                "action": "show_entity",
                "id": &*m.kind.id,
                "uuid": uuid_str,
                "name": name.component(),
            }),
        ),
        ("insertion".into(), uuid_str.into()),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::mobs::{Anim, Brain, Mob, MobType, Part, Species};
    use horizon_client::scoreboard::Scoreboard;
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    fn test_mob(id: u32, species: Species, name: &str) -> Mob {
        let part = Part {
            name: "root".into(),
            pose: [0.0; 9],
            polygons: Vec::new(),
            children: Vec::new(),
        };
        let kind = MobType {
            id: name.into(),
            species,
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            category: "creature".into(),
            max_health: 20.0,
            speed: 0.25,
            flying_speed: 0.02,
            attack_damage: 2.0,
            attack_knockback: 0.0,
            follow_range: 16.0,
            step_height: 0.6,
            armor: 0.0,
            armor_toughness: 0.0,
            knockback_resistance: 0.0,
            jump_strength: 0.42,
            attributes: Default::default(),
            item_stack_limits: Default::default(),
            gravity: 0.08,
            safe_fall_distance: 3.0,
            fall_damage_multiplier: 1.0,
            water_efficiency: 0.0,
            water_slowdown: 0.8,
            air_speed: 0.02,
            baby_scale: 0.5,
            head_rot_speed: 10.0,
            max_head_x_rot: 40.0,
            max_head_y_rot: 75.0,
            ambient_interval: 80,
            flags: Default::default(),
            model: Arc::new(part),
            layers: Default::default(),
            texture: String::new(),
            brain: Brain::Passive,
            anim: Anim::Quadruped,
            sounds: ["entity.wolf.ambient"; 3],
            step_sound: None,
            food: Default::default(),
            tempt: Default::default(),
        };
        Mob::new(id, Arc::new(kind), Vec3::ZERO, 0.0)
    }

    #[test]
    fn mob_text_includes_hover_event_and_uuid_insertion() {
        let m = test_mob(1, Species::Wolf, "minecraft:wolf");
        let uuid = [1, 2, 3, 4];
        let text = format_mob_text(&m, None, uuid);
        let comp = text.component();

        let expected_uuid = horizon_client_core::uuid::uuid_text(uuid);
        assert_eq!(comp["insertion"], expected_uuid);
        assert_eq!(comp["hover_event"]["action"], "show_entity");
        assert_eq!(comp["hover_event"]["id"], "minecraft:wolf");
        assert_eq!(comp["hover_event"]["uuid"], expected_uuid);
        assert_eq!(comp["hover_event"]["name"], m.command_name().component());
    }

    #[test]
    fn mob_text_preserves_custom_name_and_team_formatting() {
        let mut m = test_mob(1, Species::Wolf, "minecraft:wolf");
        let uuid = [5, 6, 7, 8];
        m.custom_name = Some("Ghost".into());

        let mut board = Scoreboard::default();
        let team = board.add_team("red");
        team.prefix = horizon_client::scoreboard::literal("[RED] ");

        let text = format_mob_text(&m, Some(team), uuid);
        let comp = text.component();

        let expected_uuid = horizon_client_core::uuid::uuid_text(uuid);
        assert_eq!(comp["insertion"], expected_uuid);
        assert_eq!(comp["hover_event"]["action"], "show_entity");
        assert_eq!(comp["hover_event"]["id"], "minecraft:wolf");
        assert_eq!(comp["hover_event"]["uuid"], expected_uuid);
        assert_eq!(
            comp["hover_event"]["name"],
            serde_json::json!({"text": "Ghost"})
        );
    }
}
