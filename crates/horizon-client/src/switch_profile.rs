//! The Switch's budget (docs/switch.md). A program started as a game gets about 3.1 GiB of heap
//! for everything, GPU memory included (no separate video memory, no swap), on three usable CPU
//! cores, so the first launch starts from the Performance preset with a smaller world, and what
//! the player may raise is capped to what fits. Pure functions: they apply on the Switch, and on
//! any host with `HORIZON_SWITCH_PROFILE=1`, which is how they are tested.
use crate::{
    graphics::{self, Preset},
    settings::Settings,
};

/// Render distance in chunks: a ring of 12 is 625 columns, each with its blocks, light and meshes
/// in the one heap. Raise it only after measuring `[Memory]` in log.txt.
pub const MAX_RENDER_DISTANCE: i32 = 12;
pub const MAX_SIMULATION_DISTANCE: i32 = 8;
/// Distant terrain (LODs), in chunks; off until the player turns it on, and never past this.
pub const MAX_LOD_DISTANCE: u32 = 96;
/// Chunk worker threads: the render thread and the server thread take two of the three cores the
/// program may use (the fourth also runs system work), and a spinning high-priority thread starves
/// the rest, so one worker is the safe default.
pub const WORKERS: u32 = 1;
/// Section mesh bytes uploaded per frame, KiB: GPU buffers come out of the same heap, so a big load
/// is spread over frames instead of doubling the memory at once.
pub const UPLOAD_BUDGET_KIB: u32 = 8192;

/// Whether the Switch budget applies.
pub fn active() -> bool {
    cfg!(target_os = "horizon")
        || std::env::var_os("HORIZON_SWITCH_PROFILE").is_some_and(|v| v != "0")
}

/// The settings of a first launch (no `options.txt` yet).
pub fn defaults(s: &mut Settings) {
    graphics::apply_preset(Preset::Performance, s);
    s.render_distance = 8;
    s.simulation_distance = 6;
    s.max_fps = 60;
    s.vsync = true;
    s.horizon.upload_budget_kib = UPLOAD_BUDGET_KIB;
    s.horizon.workers = WORKERS;
    s.horizon.lod_distance = 0;
}

/// What every launch enforces, whatever options.txt says.
pub fn clamp(s: &mut Settings) {
    s.render_distance = s.render_distance.min(MAX_RENDER_DISTANCE);
    s.simulation_distance = s.simulation_distance.min(MAX_SIMULATION_DISTANCE);
    let g = &mut s.horizon;
    g.lod_distance = g.lod_distance.min(MAX_LOD_DISTANCE);
    if g.upload_budget_kib == 0 {
        g.upload_budget_kib = UPLOAD_BUDGET_KIB;
    }
    if g.workers == 0 {
        g.workers = WORKERS;
    }
    // Half the pixels do not fit twice in the heap: no supersampling.
    g.resolution_scale = g.resolution_scale.min(100);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_launch_is_small_and_paced() {
        let mut s = Settings::default();
        defaults(&mut s);
        assert_eq!(
            (s.render_distance, s.simulation_distance, s.max_fps),
            (8, 6, 60)
        );
        assert_eq!((s.horizon.workers, s.horizon.lod_distance), (WORKERS, 0));
        assert_eq!(s.horizon.upload_budget_kib, UPLOAD_BUDGET_KIB);
    }

    #[test]
    fn clamp_caps_what_a_pc_options_file_asks_for() {
        let mut s = Settings::default();
        s.render_distance = 32;
        s.simulation_distance = 20;
        s.horizon.lod_distance = 512;
        s.horizon.resolution_scale = 200;
        s.horizon.workers = 0;
        s.horizon.upload_budget_kib = 0;
        clamp(&mut s);
        assert_eq!((s.render_distance, s.simulation_distance), (12, 8));
        assert_eq!(
            (s.horizon.lod_distance, s.horizon.resolution_scale),
            (96, 100)
        );
        assert_eq!((s.horizon.workers, s.horizon.upload_budget_kib), (1, 8192));
        // A smaller choice stays.
        let mut s = Settings::default();
        s.render_distance = 6;
        clamp(&mut s);
        assert_eq!(s.render_distance, 6);
    }
}
