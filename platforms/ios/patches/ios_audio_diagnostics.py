#!/usr/bin/env python3
"""Record Horizon CPAL audio-open results in Nova's exportable logs on iOS.

The backend remains entirely Horizon's. Only diagnostic messages are added in
the temporary CI checkout, so upstream audio sources stay unchanged in Git.
"""
from pathlib import Path

path = Path("crates/horizon-client-render/src/audio.rs")
source = path.read_text()
def change(old, new):
    global source
    assert source.count(old) == 1, f"audio upstream changed: {old[:85]!r}"
    source = source.replace(old, new, 1)

change("use crate::Result;\n", """use crate::Result;

#[cfg(target_os = "ios")]
fn nova_ios_audio_trace(stage: &str) {
    use std::io::Write as _;
    if let Some(path) = std::env::var_os("NOVA_DIAGNOSTIC_LOG") {
        if let Ok(mut file) = std::fs::OpenOptions::new().append(true).open(path) {
            let _ = writeln!(file, "[audio] {stage}");
        }
    }
}
""")
change('                    log::warn!("Horizon: no sound: {e}");',
       '''                    log::warn!("Horizon: no sound: {e}");
                    #[cfg(target_os = "ios")]
                    nova_ios_audio_trace(&format!("output.open.failed: {e}"));''')
change("        let mut engine = SoundEngine::new(registry);",
       '''        #[cfg(target_os = "ios")]
        nova_ios_audio_trace(&format!("output.initialized; live={live}"));
        let mut engine = SoundEngine::new(registry);''')
path.write_text(source)
print("Nova: CPAL output success or error now recorded in exported iOS diagnostics")
