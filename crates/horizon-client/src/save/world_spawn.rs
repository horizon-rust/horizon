//! The world spawn record (`level.dat` `spawn`): `LevelData.RespawnData`'s
//! global pos and angle, which the respawn level and the world spawn's
//! dimension come from. `Level` keeps the parts as fields, as it does for a
//! player's own respawn point (`respawn`).
use super::{Level, Tag, number, string, text};

/// 26.3's `spawn {pos: [I; x, y, z], dimension, yaw, pitch}`; an older world
/// keeps `SpawnX/Y/Z` and is the Overworld's.
pub(super) fn read(data: &Tag) -> (Option<[i32; 3]>, [f32; 2], String) {
    let spawn = data.get("spawn");
    let n = |t: Option<&Tag>| number(t).unwrap_or(0);
    let pos = match spawn.and_then(|s| s.get("pos")) {
        Some(Tag::IntArray(p)) if p.len() == 3 => Some([p[0], p[1], p[2]]),
        _ => data.get("SpawnX").map(|_| {
            [
                n(data.get("SpawnX")) as i32,
                n(data.get("SpawnY")) as i32,
                n(data.get("SpawnZ")) as i32,
            ]
        }),
    };
    let angle = |k: &str| match spawn.and_then(|s| s.get(k)) {
        Some(Tag::Float(b)) => f32::from_bits(*b),
        _ => 0.0,
    };
    let dimension = spawn
        .and_then(|s| text(s.get("dimension")))
        .unwrap_or_else(|| "minecraft:overworld".into());
    (pos, [angle("yaw"), angle("pitch")], dimension)
}

impl Level {
    /// The `spawn` compound as `Level::write` stores it.
    pub(super) fn spawn_tag(&self) -> Tag {
        let pos = self.world_spawn.unwrap_or([0, 64, 0]);
        let [yaw, pitch] = self.world_spawn_angle;
        // A level built in memory, not read from a file, is the Overworld's.
        let dimension = if self.world_spawn_dimension.is_empty() {
            "minecraft:overworld"
        } else {
            &self.world_spawn_dimension
        };
        Tag::compound([
            ("pos", Tag::IntArray(pos.to_vec())),
            ("dimension", string(dimension)),
            ("yaw", Tag::Float(yaw.to_bits())),
            ("pitch", Tag::Float(pitch.to_bits())),
        ])
    }
}
