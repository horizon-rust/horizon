//! Microsoft sessions handed over by the launcher; offline split profiles belong to the instance.
use horizon_client::auth::AuthSession;
use std::sync::Arc;

/// The launcher's other signed-in accounts (`HORIZON_ACCOUNT_SESSIONS`: name, uuid and a session
/// file each), for splitscreen players to sign in with: read at start beside `--session-file`
/// (`read_now`), as the files go when read and are refused once a couple of minutes old.
pub(super) fn accounts() -> Vec<Arc<AuthSession>> {
    READ.get_or_init(read).0.clone()
}

static READ: std::sync::OnceLock<(Vec<Arc<AuthSession>>, Vec<String>)> = std::sync::OnceLock::new();

pub(super) fn unavailable() -> Vec<String> {
    READ.get_or_init(read).1.clone()
}

/// Read (and so delete) the session files now, before the game loads.
pub(in crate::game) fn read_now() {
    READ.get_or_init(read);
}

fn read() -> (Vec<Arc<AuthSession>>, Vec<String>) {
    let Ok(json) = std::env::var("HORIZON_ACCOUNT_SESSIONS") else {
        return Default::default();
    };
    let list: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap_or_default();
    let mut sessions = Vec::new();
    let mut unavailable = Vec::new();
    for a in list {
        let Some(name) = a.get("name").and_then(|v| v.as_str()) else {
            continue;
        };
        if !horizon_client::split_profiles::valid_name(name) {
            continue;
        }
        let session = a
            .get("file")
            .and_then(|v| v.as_str())
            .and_then(|file| AuthSession::load(std::path::Path::new(file)).ok());
        match session {
            Some(session) if session.name.eq_ignore_ascii_case(name) => {
                sessions.push(Arc::new(session))
            }
            _ => {
                // Reasons are deliberately fixed text: never log session contents or tokens.
                log::warn!(
                    "Horizon: splitscreen: {name} is offline; sign in again in the launcher."
                );
                unavailable.push(name.to_owned());
            }
        }
    }
    (sessions, unavailable)
}
