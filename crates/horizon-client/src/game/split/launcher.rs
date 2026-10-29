//! Microsoft sessions handed over by the launcher; offline split profiles belong to the instance.
use horizon_client::auth::AuthSession;
use std::sync::Arc;

/// The launcher's other signed-in accounts (`HORIZON_ACCOUNT_SESSIONS`: name, uuid and a session
/// file each), for splitscreen players to sign in with. Read at once: the files go when read and
/// are refused once a couple of minutes old.
pub(super) fn accounts() -> Vec<Arc<AuthSession>> {
    let Ok(json) = std::env::var("HORIZON_ACCOUNT_SESSIONS") else {
        return Vec::new();
    };
    let list: Vec<serde_json::Value> = serde_json::from_str(&json).unwrap_or_default();
    list.iter()
        .filter_map(|a| a.get("file")?.as_str())
        .filter_map(|file| match AuthSession::load(std::path::Path::new(file)) {
            Ok(session) => Some(Arc::new(session)),
            Err(e) => {
                log::warn!("Horizon: splitscreen: an account's session: {e}");
                None
            }
        })
        .collect()
}
