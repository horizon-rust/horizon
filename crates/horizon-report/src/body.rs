//! The request `POST /v1/report` takes, built from what the player wrote and what the game knows
//! about the machine (docs/telemetry.md, "Bug reports from the game": the fields and their limits).
//! Nothing here touches the game or the network, so the limits are tested as plain functions.
use serde_json::{Value, json};

/// The areas a bug may name: the repository's area labels, as the server accepts them.
pub const AREAS: [&str; 10] = [
    "blocks",
    "mobs",
    "rendering",
    "audio",
    "ui",
    "multiplayer",
    "saves",
    "worldgen",
    "chunks",
    "commands",
];

pub const TITLE: std::ops::RangeInclusive<usize> = 5..=120;
pub const HAPPENED_BUG: std::ops::RangeInclusive<usize> = 10..=500;
pub const HAPPENED_PERF: std::ops::RangeInclusive<usize> = 0..=500;
pub const EXPECTED_MAX: usize = 500;
pub const VERSION: std::ops::RangeInclusive<usize> = 3..=80;
pub const MEASUREMENT: std::ops::RangeInclusive<usize> = 10..=6000;
pub const SYSTEM_ROWS: usize = 60;
pub const SETTINGS_ROWS: usize = 40;
pub const LABEL_MAX: usize = 40;
pub const VALUE_MAX: usize = 200;

/// What the report is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bug,
    Performance,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Bug => "bug",
            Self::Performance => "performance",
        }
    }
}

/// What the player chose and wrote.
#[derive(Clone, Debug)]
pub struct Draft {
    pub kind: Kind,
    /// An index into [`AREAS`]; only a bug sends it.
    pub area: usize,
    pub happened: String,
    pub expected: String,
}

impl Default for Draft {
    fn default() -> Self {
        Self {
            kind: Kind::Bug,
            area: 0,
            happened: String::new(),
            expected: String::new(),
        }
    }
}

/// What the game fills in, already scrubbed (`Scrubber`).
#[derive(Clone, Debug, Default)]
pub struct Technical {
    pub version: String,
    pub system: Vec<(String, String)>,
    pub settings: Vec<(String, String)>,
    /// The measurement's summary, once the game takes one (a performance report needs it).
    pub measurement: Option<String>,
}

/// Why a draft can't be sent yet. Each has a translation key for the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// A bug needs 10 to 500 characters of what happened.
    Happened,
    /// A performance report needs a measurement, which the game can't take yet.
    Measurement,
    /// The build's name is shorter than 3 characters.
    Version,
}

impl Problem {
    pub fn key(self) -> &'static str {
        match self {
            Self::Happened => "horizon.report.problem_happened",
            Self::Measurement => "horizon.report.problem_measurement",
            Self::Version => "horizon.report.problem_version",
        }
    }
}

/// Text as the server reads it: one line, no control characters, trimmed.
pub fn clean(text: &str) -> String {
    let one_line = text.replace("\r\n", "\n").replace('\n', " ");
    one_line
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_owned()
}

/// `text` cut to `max` characters, ending in `…` when it was cut.
pub fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

/// The issue's title, written by the game from the player's sentence: its first sentence, at most
/// 80 characters, cut at a word.
pub fn title(draft: &Draft) -> String {
    let said = clean(&draft.happened);
    if said.is_empty() {
        return "Performance problem".to_owned();
    }
    let first = said
        .char_indices()
        .find(|&(i, c)| {
            matches!(c, '.' | '!' | '?')
                && said[i + c.len_utf8()..].starts_with(' ')
                && said[..i].chars().count() >= 5
        })
        .map_or(said.as_str(), |(i, c)| &said[..i + c.len_utf8()]);
    let first = first.trim_end_matches('.');
    if first.chars().count() <= 80 {
        return first.to_owned();
    }
    let cut: String = first.chars().take(79).collect();
    // Back to the last space, unless that would leave too little.
    let at_word = match cut.rfind(' ') {
        Some(i) if cut[..i].chars().count() >= 20 => &cut[..i],
        _ => cut.as_str(),
    };
    format!("{}…", at_word.trim_end())
}

fn rows(rows: &[(String, String)], max: usize) -> Vec<Value> {
    rows.iter()
        .filter_map(|(label, value)| {
            let label = shorten(&clean(label), LABEL_MAX);
            (!label.is_empty()).then(|| json!([label, shorten(&clean(value), VALUE_MAX)]))
        })
        .take(max)
        .collect()
}

/// What still stops a draft from being sent, apart from the machine's details: a bug needs the
/// player's sentence, a performance report a measurement (which the game does not take yet, so
/// `measurement` is None until it does).
pub fn check(draft: &Draft, measurement: Option<&str>) -> Result<(), Problem> {
    let bug = draft.kind == Kind::Bug;
    let allowed = if bug { HAPPENED_BUG } else { HAPPENED_PERF };
    if !allowed.contains(&clean(&draft.happened).chars().count()) {
        return Err(Problem::Happened);
    }
    let measured = measurement.is_some_and(|m| MEASUREMENT.contains(&clean(m).chars().count()));
    if !bug && !measured {
        return Err(Problem::Measurement);
    }
    Ok(())
}

/// The request body, or what the player must still do ([`check`]).
pub fn request(draft: &Draft, technical: &Technical) -> Result<Value, Problem> {
    check(draft, technical.measurement.as_deref())?;
    let bug = draft.kind == Kind::Bug;
    let happened = clean(&draft.happened);
    let version = clean(&technical.version);
    if !VERSION.contains(&version.chars().count()) {
        return Err(Problem::Version);
    }
    let measurement = technical.measurement.as_deref().map(clean);
    let mut body = json!({
        "category": draft.kind.name(),
        "title": title(draft),
        "version": shorten(&version, *VERSION.end()),
        "system": rows(&technical.system, SYSTEM_ROWS),
    });
    let fields = body.as_object_mut().expect("a JSON object");
    if !happened.is_empty() {
        fields.insert("happened".into(), happened.into());
    }
    let expected = shorten(&clean(&draft.expected), EXPECTED_MAX);
    if !expected.is_empty() {
        fields.insert("expected".into(), expected.into());
    }
    if bug {
        fields.insert("area".into(), AREAS[draft.area % AREAS.len()].into());
    }
    let settings = rows(&technical.settings, SETTINGS_ROWS);
    if !settings.is_empty() {
        fields.insert("settings".into(), settings.into());
    }
    if let Some(measurement) = measurement.filter(|_| !bug) {
        fields.insert("measurement".into(), measurement.into());
    }
    // `system` needs at least one row: the OS is always known.
    if fields["system"].as_array().is_some_and(Vec::is_empty) {
        fields.insert("system".into(), json!([["Operating system", "unknown"]]));
    }
    Ok(body)
}

/// The issue as the player reads it before sending, from the request itself, so what they see is
/// what is sent. The server then makes each piece inert (no mentions, links or HTML).
pub fn preview(body: &Value) -> Vec<String> {
    let text = |name: &str| body[name].as_str().unwrap_or_default();
    let table = |name: &str| -> Vec<String> {
        body[name]
            .as_array()
            .map(|rows| {
                rows.iter()
                    .map(|row| {
                        format!(
                            "{}: {}",
                            row[0].as_str().unwrap_or_default(),
                            row[1].as_str().unwrap_or_default()
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut labels = vec!["from-game"];
    if text("category") == "bug" {
        labels.push("bug");
        if !text("area").is_empty() {
            labels.push(text("area"));
        }
    } else {
        labels.push("perf");
    }
    let mut lines = vec![
        format!("Title: {}", text("title")),
        format!("Labels: {}", labels.join(", ")),
    ];
    if !text("happened").is_empty() {
        lines.push(format!("What happened: {}", text("happened")));
    }
    if !text("expected").is_empty() {
        lines.push(format!("What I expected: {}", text("expected")));
    }
    lines.push(format!("Version: {}", text("version")));
    if !text("measurement").is_empty() {
        lines.push(format!("Measurement: {}", text("measurement")));
    }
    lines.push("Machine".to_owned());
    lines.extend(table("system"));
    if body["settings"].is_array() {
        lines.push("Settings".to_owned());
        lines.extend(table("settings"));
    }
    lines
}

/// `lines` broken at spaces into lines of at most `width` characters (a longer word is split).
pub fn wrap(lines: &[String], width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in lines {
        let mut current = String::new();
        for word in line.split_whitespace() {
            let mut word = word;
            // A word wider than a line is split.
            while word.chars().count() > width {
                let at = word
                    .char_indices()
                    .nth(width)
                    .map_or(word.len(), |(i, _)| i);
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
                out.push(word[..at].to_owned());
                word = &word[at..];
            }
            let joined = current.chars().count() + 1 + word.chars().count();
            if !current.is_empty() && joined > width {
                out.push(std::mem::take(&mut current));
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bug(happened: &str) -> Draft {
        Draft {
            happened: happened.to_owned(),
            ..Draft::default()
        }
    }

    fn technical() -> Technical {
        Technical {
            version: "v0.2.31".into(),
            system: vec![("CPU".into(), "Some CPU".into())],
            settings: vec![("render distance".into(), "12".into())],
            measurement: None,
        }
    }

    #[test]
    fn a_bug_carries_the_fields_the_server_names() {
        let mut draft = bug("The sky stays black after I sleep.\nIt happens every night.");
        draft.expected = "  Daylight  ".into();
        draft.area = 2;
        let body = request(&draft, &technical()).unwrap();
        assert_eq!(
            body,
            json!({
                "category": "bug",
                "title": "The sky stays black after I sleep",
                "happened": "The sky stays black after I sleep. It happens every night.",
                "expected": "Daylight",
                "area": "rendering",
                "version": "v0.2.31",
                "system": [["CPU", "Some CPU"]],
                "settings": [["render distance", "12"]],
            })
        );
    }

    #[test]
    fn a_bug_needs_ten_to_five_hundred_characters() {
        assert_eq!(
            request(&bug("too short"), &technical()),
            Err(Problem::Happened)
        );
        assert!(request(&bug("ten chars!"), &technical()).is_ok());
        assert!(request(&bug(&"a".repeat(500)), &technical()).is_ok());
        assert_eq!(
            request(&bug(&"a".repeat(501)), &technical()),
            Err(Problem::Happened)
        );
        // Spaces around it don't count.
        assert_eq!(
            request(&bug("   short   "), &technical()),
            Err(Problem::Happened)
        );
    }

    #[test]
    fn expected_is_cut_at_five_hundred_and_dropped_when_empty() {
        let mut draft = bug("The sky stays black");
        draft.expected = "  ".into();
        let body = request(&draft, &technical()).unwrap();
        assert!(body.get("expected").is_none());
        draft.expected = "e".repeat(900);
        let body = request(&draft, &technical()).unwrap();
        assert_eq!(body["expected"].as_str().unwrap().chars().count(), 500);
    }

    #[test]
    fn only_a_bug_names_an_area() {
        let mut draft = bug("The sky stays black");
        draft.area = 9;
        assert_eq!(request(&draft, &technical()).unwrap()["area"], "commands");
        // An index past the list still names one the server knows.
        draft.area = 12;
        assert_eq!(request(&draft, &technical()).unwrap()["area"], "rendering");
        for area in AREAS {
            assert!(!area.is_empty());
        }
    }

    #[test]
    fn a_performance_report_waits_for_its_measurement() {
        let draft = Draft {
            kind: Kind::Performance,
            ..Draft::default()
        };
        assert_eq!(request(&draft, &technical()), Err(Problem::Measurement));
        let mut measured = technical();
        measured.measurement = Some("p99 frame time 31 ms over 60 s".into());
        let body = request(&draft, &measured).unwrap();
        assert_eq!(body["category"], "performance");
        assert_eq!(body["title"], "Performance problem");
        assert_eq!(body["measurement"], "p99 frame time 31 ms over 60 s");
        assert!(body.get("area").is_none());
        assert!(body.get("happened").is_none());
        measured.measurement = Some("short".into());
        assert_eq!(request(&draft, &measured), Err(Problem::Measurement));
    }

    #[test]
    fn the_title_is_five_to_a_hundred_twenty_characters() {
        let long = format!("{} end", "word ".repeat(60));
        let title = title(&bug(&long));
        assert!(title.chars().count() <= 80, "{title}");
        assert!(title.ends_with('…'));
        assert!(TITLE.contains(&title.chars().count()));
        assert_eq!(
            super::title(&bug("No full stop here at all")),
            "No full stop here at all"
        );
        assert_eq!(
            super::title(&bug("Hi. And then more words follow")),
            "Hi. And then more words follow"
        );
        assert_eq!(super::title(&bug("Crash! Then the game closed")), "Crash!");
        assert_eq!(
            super::title(&bug("Water does not flow. It stays still")),
            "Water does not flow"
        );
    }

    #[test]
    fn rows_keep_to_the_server_limits() {
        let mut technical = technical();
        technical.system = (0..80)
            .map(|i| (format!("{i}{}", "l".repeat(60)), "v".repeat(300)))
            .collect();
        technical.system.push((String::new(), "no label".into()));
        let body = request(&bug("The sky stays black"), &technical).unwrap();
        let system = body["system"].as_array().unwrap();
        assert_eq!(system.len(), SYSTEM_ROWS);
        assert!(system[0][0].as_str().unwrap().chars().count() <= LABEL_MAX);
        assert!(system[0][1].as_str().unwrap().chars().count() <= VALUE_MAX);
        technical.settings = (0..60).map(|i| (format!("s{i}"), "1".into())).collect();
        let body = request(&bug("The sky stays black"), &technical).unwrap();
        assert_eq!(body["settings"].as_array().unwrap().len(), SETTINGS_ROWS);
    }

    #[test]
    fn a_machine_without_rows_still_sends_one() {
        let mut technical = technical();
        technical.system.clear();
        let body = request(&bug("The sky stays black"), &technical).unwrap();
        assert_eq!(body["system"], json!([["Operating system", "unknown"]]));
        technical.version = "v".into();
        assert_eq!(
            request(&bug("The sky stays black"), &technical),
            Err(Problem::Version)
        );
    }

    #[test]
    fn the_preview_shows_what_is_sent() {
        let mut draft = bug("The sky stays black");
        draft.expected = "Daylight".into();
        let body = request(&draft, &technical()).unwrap();
        assert_eq!(
            preview(&body),
            [
                "Title: The sky stays black",
                "Labels: from-game, bug, blocks",
                "What happened: The sky stays black",
                "What I expected: Daylight",
                "Version: v0.2.31",
                "Machine",
                "CPU: Some CPU",
                "Settings",
                "render distance: 12",
            ]
        );
    }

    #[test]
    fn lines_wrap_at_spaces_and_split_long_words() {
        let lines = vec!["one two three four".to_owned(), "x".repeat(25)];
        assert_eq!(
            wrap(&lines, 10),
            ["one two", "three four", "xxxxxxxxxx", "xxxxxxxxxx", "xxxxx"]
        );
        assert_eq!(wrap(&["".to_owned()], 10), Vec::<String>::new());
    }
}
