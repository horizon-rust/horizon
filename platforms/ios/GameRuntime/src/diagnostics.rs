//! Minimal synchronous breadcrumbs for startup failures on real iOS devices.
//! Deliberately avoids log sinks, panic hooks, stdout and networking.
use std::{fs::OpenOptions, io::Write, time::{SystemTime, UNIX_EPOCH}};

pub(crate) fn trace(stage: &str) {
    let Some(path) = std::env::var_os("NOVA_DIAGNOSTIC_LOG") else { return };
    if let Ok(mut file) = OpenOptions::new().append(true).open(path) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let _ = writeln!(file, "{stamp} [rust] {stage}");
        let _ = file.flush();
    }
}


/// Mirror the game's complete log-facade INFO/WARN/ERROR lines into the
/// current launch file. Horizon keeps its own Game/logs/latest.log unchanged.
/// This hook runs on Horizon's log writer thread, not on the render thread.
pub(crate) fn game_log(formatted: &str) {
    let Some(path) = std::env::var_os("NOVA_DIAGNOSTIC_LOG") else { return };
    if let Ok(mut file) = OpenOptions::new().append(true).open(path) {
        for line in formatted.lines() {
            let _ = writeln!(file, "[horizon] {line}");
        }
        let _ = file.flush();
    }
}
