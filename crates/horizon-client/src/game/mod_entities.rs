//! Ephemeral server entity records, kept with their loaded level.
use horizon_client::chunk_map::ChunkMap;
use horizon_mods::{
    EntityReason,
    entities::{Level, LoadedLevel, Record, WorldId},
};
use std::collections::HashSet;

#[derive(Default)]
pub(super) struct ModEntities {
    level: Level,
}
impl ModEntities {
    pub(super) fn loaded(
        &mut self,
        dimension: &str,
        min_y: i32,
        max_y: i32,
        chunks: &ChunkMap,
    ) -> LoadedLevel {
        self.level
            .loaded(dimension, min_y, max_y, chunks.keys().copied())
    }
    pub(super) fn prune(&mut self) {
        self.level.prune();
    }
    pub(super) fn clear(&mut self, reason: EntityReason) {
        self.level.clear(reason);
    }
    pub(super) fn unload(&mut self, chunks: &HashSet<(i32, i32)>) {
        self.level.unload_chunks(chunks);
    }
    pub(super) fn records(&self) -> Vec<Record> {
        self.level.records()
    }
    pub(super) fn allocated(&self) -> bool {
        self.level.allocated()
    }
}

/// Opt-in fixture evidence of applied records, after callbacks and cleanup.
pub(super) fn trace(world: Option<WorldId>, last_id: u32, levels: &[(&str, &ModEntities)]) {
    let Some(path) = std::env::var_os("HORIZON_MOD_ENTITIES_TRACE") else {
        return;
    };
    let levels: Vec<_> = levels.iter().map(|(dimension, level)| {
        let records: Vec<_> = level.records().iter().map(|r| {
            let p = &r.state.transform;
            serde_json::json!({"handle":r.handle,"engine_id":r.engine_id,"kind":r.kind.key,
                "renderer":r.kind.renderer,"public_version":r.kind.public_version,
                "position":[p.x,p.y,p.z],"rotation":[p.yaw,p.pitch],
                "public_state":r.state.public_state,"revision":r.revision})
        }).collect();
        serde_json::json!({"dimension":dimension,"allocated":level.allocated(),"records":records})
    }).collect();
    let value =
        serde_json::json!({"world":world.map(WorldId::get),"last_id":last_id,"levels":levels});
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let line = format!("{value}\n");
        let _ = file.write_all(line.as_bytes());
    }
}
