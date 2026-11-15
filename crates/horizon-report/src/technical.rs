//! The technical part of a report, which the game fills in: the build, the machine and what the
//! engine says of the graphics adapter and the settings (`horizon_platform::details`). Only what
//! the game already collects; the measurement, the log and the last seconds come with later pull
//! requests. Everything goes through the scrubber telemetry uses before the player sees it.
use crate::body::Technical;
use horizon_platform::system::{Machine, scrub::Scrubber};

/// The machine as report rows: nothing the system did not say.
pub fn machine_rows(machine: &Machine) -> Vec<(String, String)> {
    let mut rows = Vec::new();
    let mut add = |label: &str, value: String| {
        if !value.trim().is_empty() {
            rows.push((label.to_owned(), value.trim().to_owned()));
        }
    };
    add(
        "Operating system",
        format!("{} {}", machine.os, machine.os_version),
    );
    add("Architecture", machine.arch.clone());
    add("CPU", machine.cpu.clone());
    if machine.threads > 0 {
        add(
            "CPU threads",
            if machine.cores > 0 {
                format!("{} ({} cores)", machine.threads, machine.cores)
            } else {
                machine.threads.to_string()
            },
        );
    }
    if machine.ram_gb > 0 {
        add("Memory", format!("{} GB", machine.ram_gb));
    }
    add("Storage", machine.storage.to_owned());
    add("Power", machine.power.to_owned());
    add("Form", machine.form.to_owned());
    rows
}

/// The engine's lines (`horizon_platform::details::all`) as rows: the graphics adapter joins the
/// machine, the render settings are split into one row each (`render distance 12`), the packs and
/// the native mods stay one row.
pub fn detail_rows(
    details: &[(&'static str, String)],
) -> (Vec<(String, String)>, Vec<(String, String)>) {
    let (mut system, mut settings) = (Vec::new(), Vec::new());
    for (key, value) in details {
        match *key {
            "Graphics Adapter" => system.push(("Graphics adapter".to_owned(), value.clone())),
            "DirectX 12 shader compiler" => {
                system.push(("DirectX 12 shader compiler".to_owned(), value.clone()))
            }
            "Render Settings" => {
                for part in value.split(", ").filter(|p| !p.trim().is_empty()) {
                    let (label, value) = part.rsplit_once(' ').unwrap_or((part, ""));
                    settings.push((label.trim().to_owned(), value.trim().to_owned()));
                }
            }
            "Resource Packs" => settings.push(("Resource packs".to_owned(), value.clone())),
            "Native Mods" => settings.push(("Native mods".to_owned(), value.clone())),
            _ => {}
        }
    }
    (system, settings)
}

/// The technical part for `version` on `machine`, scrubbed.
pub fn gather(
    version: &str,
    machine: &Machine,
    details: &[(&'static str, String)],
    scrubber: &Scrubber,
) -> Technical {
    let (adapter, settings) = detail_rows(details);
    let mut system = machine_rows(machine);
    system.extend(adapter);
    let scrub = |rows: Vec<(String, String)>| -> Vec<(String, String)> {
        rows.into_iter()
            .map(|(label, value)| (label, scrubber.scrub(&value)))
            .collect()
    };
    Technical {
        version: version.to_owned(),
        system: scrub(system),
        settings: scrub(settings),
        measurement: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_machine_gives_only_what_the_system_said() {
        let machine = Machine {
            os: "macos".into(),
            os_version: "macOS 26.0".into(),
            cpu: "Apple M4".into(),
            threads: 10,
            cores: 10,
            ram_gb: 16,
            power: "ac",
            ..Machine::default()
        };
        let rows = machine_rows(&machine);
        assert_eq!(
            rows,
            [
                ("Operating system", "macos macOS 26.0"),
                ("CPU", "Apple M4"),
                ("CPU threads", "10 (10 cores)"),
                ("Memory", "16 GB"),
                ("Power", "ac"),
            ]
            .map(|(l, v)| (l.to_owned(), v.to_owned()))
        );
        assert!(machine_rows(&Machine::default()).is_empty());
    }

    #[test]
    fn render_settings_become_a_row_each() {
        let details = [
            ("Graphics Adapter", "GPU (Vulkan), driver 1".to_owned()),
            (
                "Render Settings",
                "render distance 12, max FPS unlimited, VSync on".to_owned(),
            ),
            ("Resource Packs", "vanilla".to_owned()),
            ("World", "My World".to_owned()),
        ];
        let (system, settings) = detail_rows(&details);
        assert_eq!(
            system,
            [(
                "Graphics adapter".to_owned(),
                "GPU (Vulkan), driver 1".to_owned()
            )]
        );
        assert_eq!(
            settings,
            [
                ("render distance", "12"),
                ("max FPS", "unlimited"),
                ("VSync", "on"),
                ("Resource packs", "vanilla"),
            ]
            .map(|(l, v)| (l.to_owned(), v.to_owned()))
        );
    }

    #[test]
    fn what_the_player_cannot_share_is_scrubbed() {
        let details = [(
            "Native Mods",
            "a mod from /Users/alex/mods at 192.168.1.4".to_owned(),
        )];
        let scrubber = Scrubber::bare().with_user("alex");
        let technical = gather("v0.2.31", &Machine::default(), &details, &scrubber);
        let value = &technical.settings[0].1;
        assert!(
            !value.contains("alex") && !value.contains("192.168"),
            "{value}"
        );
    }
}
