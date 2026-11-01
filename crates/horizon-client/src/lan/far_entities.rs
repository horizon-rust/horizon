//! Far entities (`horizon_protocol::far_entities`), the server's side: the
//! players and mobs a client is not tracking, as compact snapshots, so its
//! player sees the living world past the entity render distance. A client
//! asks with its range and count (`Feed::set`); the players go out every few
//! ticks, the mobs every few seconds, each viewer's own staggered. Every
//! entity's record is encoded once and the viewers' messages are copies of
//! the records they are sent.
use super::{entity::EntityState, guest::Guest};
use crate::fast_hash::FxHashMap;
use horizon_protocol::{
    entity_data::Value,
    far_entities::{self as wire, mob_flags, player_flags},
    packets::{configuration::CustomPayload, play::Clientbound as C},
};

/// Ticks between a viewer's players snapshots, and between its mobs snapshots.
pub const PLAYER_EVERY: u64 = 4;
pub const MOB_EVERY: u64 = 100;

/// What one client asked for.
struct Sub {
    range: u32,
    max: u32,
    /// Both snapshots go out at the next tick, due or not (just asked, or the level changed).
    fresh: bool,
    /// The level the client was last sent.
    level: i32,
}

/// The far entity feed of a hub.
pub struct Feed {
    /// The farthest any client is sent, in blocks; 0 is off.
    pub range: u32,
    subs: FxHashMap<u32, Sub>,
}

/// One entity as the feed sees it this tick: where it is and its record.
struct Seen<R> {
    id: i32,
    position: [f64; 3],
    record: R,
}

impl Feed {
    pub fn new(range: u32) -> Self {
        Self {
            range: range.min(wire::MAX_RANGE),
            subs: FxHashMap::default(),
        }
    }

    /// Whether the feed is on.
    pub fn on(&self) -> bool {
        self.range > 0
    }

    /// A client's `Settings` payload: its range and count, replaced; a range of 0 stops the feed for
    /// it. A payload that doesn't decode is ignored.
    pub fn set(&mut self, conn: u32, data: &[u8]) {
        let Ok(s) = wire::Settings::decode(data) else {
            return;
        };
        if s.range == 0 || !self.on() {
            self.subs.remove(&conn);
            return;
        }
        let sub = self.subs.entry(conn).or_insert(Sub {
            range: 0,
            max: 0,
            fresh: true,
            level: 0,
        });
        sub.range = s.range;
        sub.max = s.max;
        sub.fresh = true;
    }

    /// One tick's snapshots for the clients in the level `dimension`, after
    /// the entity tracking: `entities` are all the level's, and `kind` names
    /// a type's id in the entity type registry.
    pub fn publish(
        &mut self,
        ticks: u64,
        dimension: &str,
        guests: &[Guest],
        entities: &[&EntityState],
        mut kind: impl FnMut(&str) -> Option<i32>,
    ) {
        if self.subs.is_empty() {
            return;
        }
        self.subs
            .retain(|conn, _| guests.iter().any(|g| g.conn == *conn));
        let level = wire::level_id(dimension);
        let here = |g: &Guest| g.placed && g.dimension == dimension;
        let due = |g: &Guest, sub: &Sub, every: u64, offset: u64| {
            sub.fresh || sub.level != level || (ticks + u64::from(g.conn) * offset) % every == 0
        };
        let mut want_players = false;
        let mut want_mobs = false;
        for g in guests.iter().filter(|g| here(g)) {
            if let Some(sub) = self.subs.get(&g.conn) {
                want_players |= due(g, sub, PLAYER_EVERY, 1);
                want_mobs |= due(g, sub, MOB_EVERY, 7);
            }
        }
        if !want_players && !want_mobs {
            return;
        }
        // Each entity's record, once for every viewer.
        let mut players = Vec::new();
        let mut mobs = Vec::new();
        for e in entities {
            let flags = shared_flags(e);
            // An invisible entity shows to nobody, as far away as near.
            if flags & INVISIBLE != 0 {
                continue;
            }
            if e.kind == "minecraft:player" {
                if want_players {
                    players.push((
                        Seen {
                            id: e.id,
                            position: e.position,
                            record: wire::encode_player(&wire::Player {
                                id: e.id,
                                uuid: e.uuid,
                                position: e.position,
                                yaw: e.yaw,
                                pitch: e.pitch,
                                flags: player_flags_of(flags),
                            }),
                        },
                        (e.spectator, e.watching),
                    ));
                }
            } else if want_mobs
                && is_far_mob(e)
                && let Some(id) = kind(&e.kind).and_then(|id| u16::try_from(id).ok())
            {
                let (flags, variant) = look(e);
                mobs.push((
                    Seen {
                        id: e.id,
                        position: e.position,
                        record: wire::encode_mob(&wire::Mob {
                            id: e.id,
                            kind: id,
                            position: e.position,
                            yaw: e.yaw,
                            flags,
                            variant,
                        }),
                    },
                    (e.spectator, e.watching),
                ));
            }
        }
        for g in guests.iter().filter(|g| here(g)) {
            let Some(sub) = self.subs.get_mut(&g.conn) else {
                continue;
            };
            let range = f64::from(sub.range.min(self.range));
            let near = |at: &[f64; 3]| {
                let (dx, dz) = (at[0] - g.position[0], at[2] - g.position[2]);
                let d = dx * dx + dz * dz;
                (d <= range * range).then_some(d)
            };
            // Another player's spectator mode and camera, as `ServerPlayer.broadcastToPlayer` has it.
            let shown = |hidden: &(bool, bool)| {
                if g.spectator() { !hidden.1 } else { !hidden.0 }
            };
            let wants = |id: i32| id != g.entity_id && !g.tracker.is_tracking(id);
            if due(g, sub, PLAYER_EVERY, 1) {
                let mut list: Vec<(f64, &[u8; wire::PLAYER_LEN])> = players
                    .iter()
                    .filter(|(s, hidden)| wants(s.id) && shown(hidden))
                    .filter_map(|(s, _)| Some((near(&s.position)?, &s.record)))
                    .collect();
                list.sort_by(|a, b| a.0.total_cmp(&b.0));
                list.truncate(wire::MAX_PLAYERS);
                let records: Vec<&[u8; wire::PLAYER_LEN]> = list.into_iter().map(|r| r.1).collect();
                send(g, wire::players_payload(level, &records));
            }
            if due(g, sub, MOB_EVERY, 7) {
                let mut list: Vec<(f64, &[u8; wire::MOB_LEN])> = mobs
                    .iter()
                    .filter(|(s, hidden)| wants(s.id) && shown(hidden))
                    .filter_map(|(s, _)| Some((near(&s.position)?, &s.record)))
                    .collect();
                list.sort_by(|a, b| a.0.total_cmp(&b.0));
                list.truncate((sub.max as usize).min(wire::MAX_MOBS));
                let records: Vec<&[u8; wire::MOB_LEN]> = list.into_iter().map(|r| r.1).collect();
                send(g, wire::mobs_payload(level, &records));
            }
            sub.fresh = false;
            sub.level = level;
        }
    }
}

fn send(g: &Guest, data: Vec<u8>) {
    g.send(&C::CustomPayload(CustomPayload {
        channel: wire::CHANNEL.into(),
        data,
    }));
}

/// `Entity.DATA_SHARED_FLAGS_ID` (entry 0): on fire 1, crouching 2, sprinting 8, swimming 16,
/// invisible 32, glowing 64, gliding 128.
fn shared_flags(e: &EntityState) -> u8 {
    e.meta
        .iter()
        .find_map(|(index, _, value)| match (index, value) {
            (0, Value::Byte(flags)) => Some(*flags as u8),
            _ => None,
        })
        .unwrap_or(0)
}
const INVISIBLE: u8 = 32;

fn player_flags_of(shared: u8) -> u8 {
    let bit = |mask: u8, flag: u8| if shared & mask != 0 { flag } else { 0 };
    bit(2, player_flags::SNEAKING)
        | bit(8, player_flags::SPRINTING)
        | bit(16, player_flags::SWIMMING)
        | bit(128, player_flags::GLIDING)
}

/// A mob the feed sends: a living entity the clients draw as a mob. Armor stands, mannequins and the
/// dragon (which has its own renderer) are left out.
fn is_far_mob(e: &EntityState) -> bool {
    !e.attributes.is_empty()
        && !matches!(
            e.kind.as_str(),
            "minecraft:armor_stand" | "minecraft:mannequin" | "minecraft:ender_dragon"
        )
}

/// A mob's look flags and variant, from its synched data.
fn look(e: &EntityState) -> (u8, u8) {
    let entry = |index: u8| e.meta.iter().find(|m| m.0 == index).map(|m| &m.2);
    let mut flags = 0;
    if let Some(baby) = baby_index(&e.kind)
        && matches!(entry(baby), Some(Value::Bool(true)))
    {
        flags |= mob_flags::BABY;
    }
    let mut variant = 0;
    match (e.kind.as_str(), entry(18)) {
        // Sheep.DATA_WOOL_ID: the colour in the low nibble, 16 sheared.
        ("minecraft:sheep", Some(Value::Byte(wool))) => {
            variant = (*wool & 15) as u8;
            if wool & 16 != 0 {
                flags |= mob_flags::SHEARED;
            }
        }
        // Fox.DATA_TYPE_ID and MushroomCow.DATA_TYPE.
        ("minecraft:fox" | "minecraft:mooshroom", Some(Value::Int(kind))) => {
            variant = (*kind).clamp(0, 1) as u8;
        }
        _ => {}
    }
    (flags, variant)
}

/// The SynchedEntityData index of the baby flag: AgeableMob.DATA_BABY_ID and
/// Zombie.DATA_BABY_ID are both the first entry after Mob's (16); Piglin's
/// comes after AbstractPiglin's zombification flag (17).
pub fn baby_index(kind: &str) -> Option<u8> {
    match kind {
        "minecraft:piglin" => Some(17),
        "minecraft:cow"
        | "minecraft:mooshroom"
        | "minecraft:pig"
        | "minecraft:sheep"
        | "minecraft:chicken"
        | "minecraft:horse"
        | "minecraft:donkey"
        | "minecraft:mule"
        | "minecraft:skeleton_horse"
        | "minecraft:zombie_horse"
        | "minecraft:llama"
        | "minecraft:trader_llama"
        | "minecraft:cat"
        | "minecraft:wolf"
        | "minecraft:fox"
        | "minecraft:rabbit"
        | "minecraft:panda"
        | "minecraft:polar_bear"
        | "minecraft:bee"
        | "minecraft:frog"
        | "minecraft:sniffer"
        | "minecraft:armadillo"
        | "minecraft:villager"
        | "minecraft:zombie"
        | "minecraft:husk"
        | "minecraft:drowned"
        | "minecraft:zombie_villager"
        | "minecraft:zombified_piglin"
        | "minecraft:hoglin"
        | "minecraft:zoglin"
        | "minecraft:goat"
        | "minecraft:ocelot"
        | "minecraft:turtle"
        | "minecraft:strider"
        | "minecraft:camel"
        | "minecraft:axolotl"
        | "minecraft:dolphin"
        | "minecraft:squid"
        | "minecraft:glow_squid"
        | "minecraft:sulfur_cube" => Some(16),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_protocol::{entity_data::serializer, packets::play::AttributeSnapshot};

    fn attribute() -> AttributeSnapshot {
        AttributeSnapshot {
            attribute: 0,
            base: 1.0,
            modifiers: Vec::new(),
        }
    }

    fn state(kind: &str, meta: Vec<(u8, i32, Value)>) -> EntityState {
        EntityState {
            kind: kind.into(),
            meta,
            ..Default::default()
        }
    }

    #[test]
    fn a_sheeps_wool_and_a_calf_show_in_the_look() {
        let sheep = state(
            "minecraft:sheep",
            vec![
                (16, serializer::BOOLEAN, Value::Bool(true)),
                (18, serializer::BYTE, Value::Byte(16 | 11)),
            ],
        );
        assert_eq!(
            look(&sheep),
            (mob_flags::BABY | mob_flags::SHEARED, 11),
            "a sheared blue lamb"
        );
        // Creeper entry 17 is "powered", not a baby flag.
        let creeper = state(
            "minecraft:creeper",
            vec![(17, serializer::BOOLEAN, Value::Bool(true))],
        );
        assert_eq!(look(&creeper), (0, 0));
    }

    #[test]
    fn shared_flags_pick_the_players_pose_and_who_is_hidden() {
        let flags = 2 | 8 | 128 | INVISIBLE;
        let player = state(
            "minecraft:player",
            vec![(0, serializer::BYTE, Value::Byte(flags as i8))],
        );
        let shared = shared_flags(&player);
        assert_eq!(
            player_flags_of(shared),
            player_flags::SNEAKING | player_flags::SPRINTING | player_flags::GLIDING
        );
        assert_ne!(shared & INVISIBLE, 0);
        assert_eq!(shared_flags(&state("minecraft:cow", Vec::new())), 0);
    }

    #[test]
    fn settings_start_and_stop_a_feed() {
        let mut feed = Feed::new(2048);
        let on = wire::Settings {
            range: 1000,
            max: 50,
        }
        .encode();
        feed.set(3, &on);
        assert_eq!(feed.subs.len(), 1);
        feed.set(3, &[0xff]);
        assert_eq!(feed.subs.len(), 1, "a bad payload changes nothing");
        feed.set(3, &wire::Settings { range: 0, max: 0 }.encode());
        assert!(feed.subs.is_empty());
        // A server with the feed off takes no subscribers.
        let mut off = Feed::new(0);
        off.set(3, &on);
        assert!(off.subs.is_empty());
    }

    #[test]
    fn mobs_are_living_entities_the_client_draws() {
        let mut living = state("minecraft:zombie", Vec::new());
        assert!(!is_far_mob(&living));
        living.attributes.push(attribute());
        assert!(is_far_mob(&living));
        let mut stand = state("minecraft:armor_stand", Vec::new());
        stand.attributes.push(attribute());
        assert!(!is_far_mob(&stand));
    }
}
