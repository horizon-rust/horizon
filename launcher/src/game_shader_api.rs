//! The shader API lines the installed game serves, as the game says itself: `horizon-client
//! --launch-api`, `content.shader.api` (docs/api/launch.md). The launcher and the game it
//! installed are separate builds, so a registry shader pack is offered, checked and marked by
//! the game's lines, never by the line this launcher was built with (#17527).
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::SystemTime;

/// A game's answer by its version folder and the time its install finished (a developer build
/// is installed again under the same name); None when it gave no answer.
type Asked = BTreeMap<(PathBuf, Option<SystemTime>), Option<Vec<String>>>;
static ASKED: Mutex<Asked> = Mutex::new(BTreeMap::new());

/// The shader API lines served by the game the selected instance plays. With no game installed
/// yet, or one that doesn't say, the line this launcher's own build serves.
pub fn served() -> Vec<String> {
    #[cfg(test)]
    if let Some(lines) = tests::GAME.with(|game| game.borrow().clone()) {
        return lines;
    }
    asked().unwrap_or_else(|| vec![horizon_shader_api::API.to_owned()])
}

fn asked() -> Option<Vec<String>> {
    // A test never starts a game.
    if cfg!(test) {
        return None;
    }
    let tag = crate::instances::selected().resolve_version(None).ok()?;
    let dir = crate::backend::version_dir(&tag);
    let installed = std::fs::metadata(dir.join(".installed")).and_then(|m| m.modified());
    // Held while the game answers: everyone who asks meanwhile waits for the one answer.
    let mut asked = ASKED.lock().unwrap_or_else(|e| e.into_inner());
    asked
        .entry((dir, installed.ok()))
        .or_insert_with(|| ask(&tag))
        .clone()
}

/// Asks the installed game `tag`. It prints the reply and ends before it opens a log or a
/// window.
fn ask(tag: &str) -> Option<Vec<String>> {
    let output = crate::process::command(crate::backend::client_program(tag))
        .arg("--launch-api")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then(|| lines_of(&output.stdout))?
}

/// `content.shader.api` of a `--launch-api` reply.
fn lines_of(reply: &[u8]) -> Option<Vec<String>> {
    #[derive(serde::Deserialize)]
    struct Reply {
        content: Content,
    }
    #[derive(serde::Deserialize)]
    struct Content {
        shader: Shader,
    }
    #[derive(serde::Deserialize)]
    struct Shader {
        api: Vec<String>,
    }
    let reply: Reply = serde_json::from_slice(reply).ok()?;
    Some(reply.content.shader.api)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        pub(super) static GAME: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    }

    /// Runs `run` as if the installed game served the shader API lines `lines`.
    pub fn with_game<T>(lines: &[&str], run: impl FnOnce() -> T) -> T {
        let lines = lines.iter().map(|line| (*line).to_owned()).collect();
        GAME.with(|game| *game.borrow_mut() = Some(lines));
        let out = run();
        GAME.with(|game| *game.borrow_mut() = None);
        out
    }

    #[test]
    fn reads_the_lines_a_game_says_it_serves() {
        // The Nightly of 2026-11-13 (9f42064c11) and a game from main after #15886.
        let nightly =
            br#"{"launch_api": "1.3", "content": {"shader": {"schema": 1, "api": ["0.1"]}}}"#;
        assert_eq!(lines_of(nightly).unwrap(), ["0.1"]);
        let main =
            br#"{"content": {"mod": {"api": ["0.2"]}, "shader": {"schema": 1, "api": ["0.2"]}}}"#;
        assert_eq!(lines_of(main).unwrap(), ["0.2"]);
        let both = br#"{"content": {"shader": {"api": ["0.1", "0.2"]}}}"#;
        assert_eq!(lines_of(both).unwrap(), ["0.1", "0.2"]);
        // A game from before `content`, or one that says something else: no answer.
        for reply in [
            b"not json".as_slice(),
            br#"{"launch_api": "1.0", "options": []}"#,
            br#"{"content": {"shader": {"schema": 1}}}"#,
            br#"{"content": {"shader": {"api": "0.1"}}}"#,
        ] {
            assert_eq!(lines_of(reply), None);
        }
        // Without an installed game that answers, the launcher's own line stands in.
        assert_eq!(served(), [horizon_shader_api::API]);
        assert_eq!(with_game(&["0.1"], served), ["0.1"]);
    }
}
