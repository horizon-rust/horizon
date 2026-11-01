//! Patch notes: the project's releases, as the launcher's Patch notes page shows them.

use serde_json::Value;
use std::time::Duration;

pub const RELEASES: &str = "https://api.github.com/repos/horizon-rust/horizon/releases?per_page=8";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub title: String,
    pub body: String,
}

/// The releases JSON as notes: drafts left out, the tag when a release has no name, markdown
/// reduced to plain lines.
pub fn parse(json: &Value) -> Vec<Note> {
    json.as_array()
        .into_iter()
        .flatten()
        .filter(|r| r.get("draft").and_then(Value::as_bool) != Some(true))
        .filter_map(|r| {
            let tag = r.get("tag_name").and_then(Value::as_str)?;
            let name = r
                .get("name")
                .and_then(Value::as_str)
                .filter(|n| !n.trim().is_empty())
                .unwrap_or(tag);
            let body = r.get("body").and_then(Value::as_str).unwrap_or("");
            Some(Note {
                title: name.trim().to_owned(),
                body: plain(body),
            })
        })
        .collect()
}

/// Markdown as plain lines: headings and emphasis marks dropped, bullets kept as dashes,
/// blank runs collapsed.
pub fn plain(markdown: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for raw in markdown.replace("\r\n", "\n").lines() {
        let line = raw.trim();
        let line = line.trim_start_matches('#').trim();
        let line = if let Some(rest) = line.strip_prefix("* ").or_else(|| line.strip_prefix("- ")) {
            format!("- {rest}")
        } else {
            line.to_owned()
        };
        let line = line.replace("**", "").replace('`', "");
        if line.is_empty() && out.last().is_none_or(String::is_empty) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out.join("\n")
}

/// Fetches the releases.
pub fn fetch() -> Result<Vec<Note>, String> {
    let response = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .user_agent("horizon-mobile")
        .try_proxy_from_env(true)
        .build()
        .get(RELEASES)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|_| {
            "Could not load the patch notes. Check your internet connection.".to_string()
        })?;
    let json: Value = response
        .into_json()
        .map_err(|_| "The patch notes were not readable.".to_string())?;
    let notes = parse(&json);
    if notes.is_empty() {
        Err("There are no releases to show yet.".into())
    } else {
        Ok(notes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_releases_and_skips_drafts() {
        let j = json!([
            {"tag_name":"v1","name":"Horizon 1","body":"## New\n* **Fast** `thing`\n\n\n- more","draft":false},
            {"tag_name":"v0","name":"","body":null},
            {"tag_name":"vd","name":"Draft","body":"x","draft":true}
        ]);
        let notes = parse(&j);
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].title, "Horizon 1");
        assert_eq!(notes[0].body, "New\n- Fast thing\n\n- more");
        assert_eq!(notes[1].title, "v0");
        assert_eq!(notes[1].body, "");
    }

    #[test]
    fn non_arrays_give_nothing() {
        assert!(parse(&json!({"message":"rate limited"})).is_empty());
    }
}
