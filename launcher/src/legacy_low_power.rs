use super::*;

/// Enable the Linux compatibility profile before any launcher GPU initialization.
pub(super) fn force_legacy_low_power_mode() {
    #[cfg(target_os = "linux")]
    std::env::set_var("WGPU_BACKEND", "gl");

    // Keep the entire launcher's UI on the low-cost path on every platform. The GL
    // backend selection is Linux-only; reduced motion is also useful on older machines.
    std::env::set_var("HORIZON_LAUNCHER_MOTION", "off");

    // These are launcher-owned appearance/performance preferences. Persisting them makes
    // the existing performance-mode rendering path use solid plates, zero blur, and calm
    // controls instead of merely setting the mode for the first frame.
    let mut settings = Settings::load();
    let changed = settings.performance != Some(true)
        || settings.blur != Some(0.)
        || settings.effects != horizon_launcher::effects::Effects::CALM
        || settings.reduce_motion != Some(true)
        || !settings.skip_opening;
    if changed {
        settings.performance = Some(true);
        settings.blur = Some(0.);
        settings.effects = horizon_launcher::effects::Effects::CALM;
        settings.reduce_motion = Some(true);
        settings.skip_opening = true;
        if let Err(error) = settings.save() {
            eprintln!("Horizon launcher: could not save low-effects settings: {error}");
        }
    }
}
