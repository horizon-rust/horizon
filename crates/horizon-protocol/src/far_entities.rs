//! Far entities: players and mobs past a Horizon server's normal tracking
//! range, as compact snapshots on one custom payload channel, so a Horizon
//! client can keep the living world in view far beyond the entity render
//! distance (docs/hosting.md, "Seeing further: far entities").
//!
//! The server announces the channel and its largest range in the
//! configuration phase ([`Hello`]); a vanilla client ignores it. A Horizon
//! client that hears it sends [`Settings`] in the play state (range 0 turns the
//! feed off) and is then sent snapshots of the entities it is not tracking:
//! the players every few ticks, the mobs every few seconds. Each snapshot
//! replaces the last one of its kind, so an entity that left the list is
//! gone. A snapshot is one kind of record, all of the same fixed size, so a
//! server encodes each entity once and puts the same bytes in every viewer's
//! message.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
};

/// The custom payload channel, both ways.
pub const CHANNEL: &str = "horizon:far_entities";
/// The format's version, first in every payload. A payload of another
/// version is ignored.
pub const VERSION: u8 = 1;
/// The range a server sends to by default, in blocks, and the most it ever does.
pub const DEFAULT_RANGE: u32 = 2048;
pub const MAX_RANGE: u32 = 4000;
/// Records in one snapshot at most.
pub const MAX_MOBS: usize = 4000;
pub const MAX_PLAYERS: usize = 1024;
/// Bytes of one record: a mob's and a player's.
pub const MOB_LEN: usize = 21;
pub const PLAYER_LEN: usize = 35;
/// Positions travel in this many steps to the block (a coordinate of a 30 million block
/// world fits an `i32`).
const STEPS: f64 = 16.0;

/// A player's body flags (the shared entity flags that show at a distance).
pub mod player_flags {
    pub const SNEAKING: u8 = 1;
    pub const SPRINTING: u8 = 2;
    pub const SWIMMING: u8 = 4;
    pub const GLIDING: u8 = 8;
    pub const SLEEPING: u8 = 16;
}

/// A mob's look flags.
pub mod mob_flags {
    pub const BABY: u8 = 1;
    pub const SHEARED: u8 = 2;
}

/// What a server tells a client in the configuration phase: it feeds far
/// entities, up to `range` blocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hello {
    pub range: u32,
}

impl Hello {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(VERSION);
        w.varint(self.range.min(MAX_RANGE) as i32);
        w.data
    }
    /// The announcement, or an error for another version.
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        version(&mut r)?;
        let range = r.varint()?;
        if !(0..=MAX_RANGE as i32).contains(&range) {
            return Err(Error::new("far entities: range out of bounds"));
        }
        Ok(Self {
            range: range as u32,
        })
    }
}

/// What a client asks for: entities up to `range` blocks away (0: none), at most `max` of the mobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub range: u32,
    pub max: u32,
}

impl Settings {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(VERSION);
        w.varint(self.range.min(MAX_RANGE) as i32);
        w.varint(self.max.min(MAX_MOBS as u32) as i32);
        w.data
    }
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        version(&mut r)?;
        let range = r.varint()?;
        let max = r.varint()?;
        if !(0..=MAX_RANGE as i32).contains(&range) || !(0..=MAX_MOBS as i32).contains(&max) {
            return Err(Error::new("far entities: setting out of bounds"));
        }
        Ok(Self {
            range: range as u32,
            max: max as u32,
        })
    }
}

fn version(r: &mut Reader) -> Result<()> {
    if r.u8()? != VERSION {
        return Err(Error::new("far entities: another version"));
    }
    Ok(())
}

/// A far player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Player {
    pub id: i32,
    pub uuid: u128,
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub flags: u8,
}

/// A far mob: its type is its id in the `minecraft:entity_type` registry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mob {
    pub id: i32,
    pub kind: u16,
    pub position: [f64; 3],
    pub yaw: f32,
    pub flags: u8,
    /// The look the type has a choice of (a sheep's wool colour, a fox's or a
    /// mooshroom's variant); 0 elsewhere.
    pub variant: u8,
}

fn quantize(v: f64) -> i32 {
    (v * STEPS)
        .round()
        .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
}
fn dequantize(v: i32) -> f64 {
    f64::from(v) / STEPS
}
fn pack_yaw(degrees: f32) -> u8 {
    ((degrees.rem_euclid(360.0) / 360.0 * 256.0).round() as i32 & 255) as u8
}
fn unpack_yaw(byte: u8) -> f32 {
    f32::from(byte) * 360.0 / 256.0
}
fn pack_pitch(degrees: f32) -> i8 {
    (degrees.clamp(-90.0, 90.0) / 90.0 * 127.0).round() as i8
}
fn unpack_pitch(byte: i8) -> f32 {
    f32::from(byte) / 127.0 * 90.0
}

/// A player's record, the same bytes for every viewer.
pub fn encode_player(p: &Player) -> [u8; PLAYER_LEN] {
    let mut w = Writer::new();
    w.i32(p.id);
    w.uuid(p.uuid);
    for v in p.position {
        w.i32(quantize(v));
    }
    w.u8(pack_yaw(p.yaw));
    w.i8(pack_pitch(p.pitch));
    w.u8(p.flags);
    let mut out = [0; PLAYER_LEN];
    out.copy_from_slice(&w.data);
    out
}

/// A mob's record, the same bytes for every viewer.
pub fn encode_mob(m: &Mob) -> [u8; MOB_LEN] {
    let mut w = Writer::new();
    w.i32(m.id);
    w.u16(m.kind);
    for v in m.position {
        w.i32(quantize(v));
    }
    w.u8(pack_yaw(m.yaw));
    w.u8(m.flags);
    w.u8(m.variant);
    let mut out = [0; MOB_LEN];
    out.copy_from_slice(&w.data);
    out
}

/// One snapshot, as a client reads it. `level` names the dimension (see
/// [`level_id`]): a client forgets what it remembers when it changes.
#[derive(Clone, Debug, PartialEq)]
pub enum Snapshot {
    Players { level: i32, players: Vec<Player> },
    Mobs { level: i32, mobs: Vec<Mob> },
}

/// What a dimension's name is called in a snapshot.
pub fn level_id(dimension: &str) -> i32 {
    // FNV-1a: stable between runs, unlike the standard hasher.
    let mut h: u32 = 0x811c_9dc5;
    for b in dimension.bytes() {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    h as i32
}

const PLAYERS: u8 = 0;
const MOBS: u8 = 1;

fn assemble(kind: u8, level: i32, count: usize, records: &[&[u8]], len: usize) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(VERSION);
    w.u8(kind);
    w.i32(level);
    w.u16(count as u16);
    let mut data = w.data;
    data.reserve(count * len);
    for record in records {
        data.extend_from_slice(record);
    }
    data
}

/// A players snapshot from records made with [`encode_player`].
pub fn players_payload(level: i32, records: &[&[u8; PLAYER_LEN]]) -> Vec<u8> {
    let records: Vec<&[u8]> = records.iter().map(|r| &r[..]).collect();
    assemble(PLAYERS, level, records.len(), &records, PLAYER_LEN)
}

/// A mobs snapshot from records made with [`encode_mob`].
pub fn mobs_payload(level: i32, records: &[&[u8; MOB_LEN]]) -> Vec<u8> {
    let records: Vec<&[u8]> = records.iter().map(|r| &r[..]).collect();
    assemble(MOBS, level, records.len(), &records, MOB_LEN)
}

/// A snapshot's payload; an error for one of another version, a kind this version
/// doesn't have, a count over the caps or a length that doesn't match it.
pub fn decode(data: &[u8]) -> Result<Snapshot> {
    let mut r = Reader::new(data);
    version(&mut r)?;
    let kind = r.u8()?;
    let level = r.i32()?;
    let count = usize::from(r.u16()?);
    let position = |r: &mut Reader| -> Result<[f64; 3]> {
        Ok([
            dequantize(r.i32()?),
            dequantize(r.i32()?),
            dequantize(r.i32()?),
        ])
    };
    match kind {
        PLAYERS => {
            if count > MAX_PLAYERS || r.remaining() != count * PLAYER_LEN {
                return Err(Error::new("far entities: bad players length"));
            }
            let players = (0..count)
                .map(|_| {
                    Ok(Player {
                        id: r.i32()?,
                        uuid: r.uuid()?,
                        position: position(&mut r)?,
                        yaw: unpack_yaw(r.u8()?),
                        pitch: unpack_pitch(r.i8()?),
                        flags: r.u8()?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Snapshot::Players { level, players })
        }
        MOBS => {
            if count > MAX_MOBS || r.remaining() != count * MOB_LEN {
                return Err(Error::new("far entities: bad mobs length"));
            }
            let mobs = (0..count)
                .map(|_| {
                    Ok(Mob {
                        id: r.i32()?,
                        kind: r.u16()?,
                        position: position(&mut r)?,
                        yaw: unpack_yaw(r.u8()?),
                        flags: r.u8()?,
                        variant: r.u8()?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(Snapshot::Mobs { level, mobs })
        }
        _ => Err(Error::new("far entities: unknown kind")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mob(id: i32) -> Mob {
        Mob {
            id,
            kind: 97,
            position: [-1234.5, 70.0, 30_000_000.25],
            yaw: 90.0,
            flags: mob_flags::BABY,
            variant: 11,
        }
    }

    #[test]
    fn records_have_their_sizes() {
        assert_eq!(encode_mob(&mob(1)).len(), MOB_LEN);
        let p = Player {
            id: 5,
            uuid: 7,
            position: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            flags: 0,
        };
        assert_eq!(encode_player(&p).len(), PLAYER_LEN);
    }

    #[test]
    fn mobs_round_trip_within_the_steps() {
        let (a, b) = (mob(1), mob(-2));
        let (ea, eb) = (encode_mob(&a), encode_mob(&b));
        let Snapshot::Mobs { level, mobs } = decode(&mobs_payload(7, &[&ea, &eb])).unwrap() else {
            panic!("not mobs");
        };
        assert_eq!(level, 7);
        assert_eq!(mobs.len(), 2);
        assert_eq!(mobs[0].id, 1);
        assert_eq!(mobs[1].id, -2);
        assert_eq!(mobs[0].kind, 97);
        assert_eq!(mobs[0].position, a.position);
        assert!((mobs[0].yaw - 90.0).abs() <= 360.0 / 256.0);
        assert_eq!((mobs[0].flags, mobs[0].variant), (mob_flags::BABY, 11));
    }

    #[test]
    fn players_round_trip() {
        let p = Player {
            id: 9,
            uuid: 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef,
            position: [100.5, -64.0, -0.0625],
            yaw: -90.0,
            pitch: 45.0,
            flags: player_flags::SNEAKING | player_flags::GLIDING,
        };
        let Snapshot::Players { players, .. } =
            decode(&players_payload(1, &[&encode_player(&p)])).unwrap()
        else {
            panic!("not players");
        };
        let q = players[0];
        assert_eq!((q.id, q.uuid, q.flags), (9, p.uuid, p.flags));
        assert_eq!(q.position, p.position);
        // -90 wraps to 270.
        assert!((q.yaw - 270.0).abs() <= 360.0 / 256.0);
        assert!((q.pitch - 45.0).abs() <= 90.0 / 127.0);
    }

    #[test]
    fn a_payload_that_breaks_a_rule_is_refused() {
        let e = encode_mob(&mob(1));
        let good = mobs_payload(0, &[&e]);
        assert!(decode(&good).is_ok());
        // Another version, an unknown kind, a cut record, a record too many.
        let mut other = good.clone();
        other[0] = VERSION + 1;
        assert!(decode(&other).is_err());
        let mut kind = good.clone();
        kind[1] = 9;
        assert!(decode(&kind).is_err());
        assert!(decode(&good[..good.len() - 1]).is_err());
        let mut long = good.clone();
        long.push(0);
        assert!(decode(&long).is_err());
        assert!(decode(&[]).is_err());
    }

    #[test]
    fn settings_and_hello_round_trip_and_clamp() {
        let s = Settings {
            range: 2048,
            max: 1500,
        };
        assert_eq!(Settings::decode(&s.encode()).unwrap(), s);
        let big = Settings {
            range: 99_999,
            max: 99_999,
        };
        assert_eq!(
            Settings::decode(&big.encode()).unwrap(),
            Settings {
                range: MAX_RANGE,
                max: MAX_MOBS as u32
            }
        );
        assert_eq!(
            Hello::decode(&Hello { range: 2048 }.encode())
                .unwrap()
                .range,
            2048
        );
        assert!(Hello::decode(&[VERSION + 1, 0]).is_err());
        assert!(Settings::decode(&[VERSION]).is_err());
    }

    #[test]
    fn level_ids_differ_by_dimension_and_stay_put() {
        assert_ne!(
            level_id("minecraft:overworld"),
            level_id("minecraft:the_nether")
        );
        assert_eq!(
            level_id("minecraft:overworld"),
            level_id("minecraft:overworld")
        );
    }
}
