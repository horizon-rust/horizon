//! The signed-in account, kept in one small file in the app's private folder
//! (Android gives each app its own; there is no keychain). Only the Microsoft
//! refresh token is secret; it never leaves this file except to Microsoft.

use std::path::Path;

#[derive(Clone, PartialEq, Eq)]
pub struct SavedAccount {
    pub name: String,
    pub uuid: String,
    pub refresh_token: String,
}

impl std::fmt::Debug for SavedAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedAccount")
            .field("name", &self.name)
            .field("uuid", &self.uuid)
            .finish_non_exhaustive()
    }
}

pub fn load(path: &Path) -> Option<SavedAccount> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let get = |k: &str| v.get(k)?.as_str().map(str::to_owned);
    Some(SavedAccount {
        name: get("name")?,
        uuid: get("uuid")?,
        refresh_token: get("refresh_token").filter(|t| !t.is_empty())?,
    })
}

pub fn save(path: &Path, account: &SavedAccount) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::json!({
        "name": account.name,
        "uuid": account.uuid,
        "refresh_token": account.refresh_token,
    })
    .to_string();
    // Written whole through a temporary file, readable by the app only.
    let tmp = path.with_extension("tmp");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(text.as_bytes())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&tmp, text)?;
    std::fs::rename(tmp, path)
}

/// The session file the game reads to join online-mode servers (`HORIZON_SESSION_FILE`, docs/api/launch.md):
/// `{"name", "uuid", "access_token", "user_type": "msa"}`. Readable by the app only; the game reads it once
/// and deletes it, and refuses one older than two minutes, so it is written just before the game starts.
pub fn write_session(
    path: &Path,
    name: &str,
    uuid: &str,
    access_token: &str,
) -> std::io::Result<()> {
    let text = serde_json::json!({
        "name": name,
        "uuid": uuid,
        "access_token": access_token,
        "user_type": "msa",
    })
    .to_string();
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::remove_file(path);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(text.as_bytes())
    }
    #[cfg(not(unix))]
    std::fs::write(path, text)
}

pub fn forget(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// The Azure app (client ID) for the sign-in: `HORIZON_MSA_CLIENT_ID` at run time, then at
/// build time, then the first line of `msa_client_id.txt` in `folder`. None when unset.
pub fn client_id(folder: &Path) -> Option<String> {
    std::env::var("HORIZON_MSA_CLIENT_ID")
        .ok()
        .or_else(|| option_env!("HORIZON_MSA_CLIENT_ID").map(str::to_owned))
        .or_else(|| {
            std::fs::read_to_string(folder.join("msa_client_id.txt"))
                .ok()
                .and_then(|t| t.lines().next().map(str::to_owned))
        })
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
}

/// The Minecraft launcher's own app ID, the one `prismarine-auth` and other bots sign in as. Using
/// it is the player's explicit choice (the launcher screen says what it means); it is never a default.
pub const MINECRAFT_APP_ID: &str = "00000000402b5328";

/// Whether `text` looks like an app ID: an Azure application (client) ID (a UUID, 8-4-4-4-12 hex
/// digits) or one of the Minecraft apps' own 16-digit hex IDs.
pub fn valid_client_id(text: &str) -> bool {
    crate::msa::is_live_id(text) || valid_azure_id(text)
}

fn valid_azure_id(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    parts.len() == 5
        && [8, 4, 4, 4, 12]
            .iter()
            .zip(&parts)
            .all(|(n, p)| p.len() == *n && p.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Saves a client ID the player pasted, to `msa_client_id.txt` in `folder`.
pub fn save_client_id(folder: &Path, id: &str) -> Result<String, String> {
    let id = id.trim();
    if !valid_client_id(id) {
        return Err(
            "That isn't an Azure client ID (it looks like 12345678-abcd-1234-abcd-1234567890ab)."
                .into(),
        );
    }
    std::fs::create_dir_all(folder).map_err(|e| format!("Could not save it: {e}"))?;
    std::fs::write(folder.join("msa_client_id.txt"), format!("{id}\n"))
        .map_err(|e| format!("Could not save it: {e}"))?;
    Ok(id.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_debug_hides_the_token() {
        let dir = std::env::temp_dir().join(format!("hb-store-{}", std::process::id()));
        let path = dir.join("account.json");
        let a = SavedAccount {
            name: "Steve".into(),
            uuid: "00000000000000000000000000000001".into(),
            refresh_token: "secret-token".into(),
        };
        save(&path, &a).unwrap();
        assert_eq!(load(&path), Some(a.clone()));
        assert!(!format!("{a:?}").contains("secret-token"));
        forget(&path);
        assert_eq!(load(&path), None);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_session_file_has_what_the_game_reads_and_is_private() {
        let dir = std::env::temp_dir().join(format!("hb-session-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("session.json");
        write_session(&path, "Steve", "0123456789abcdef0123456789abcdef", "TOKEN").unwrap();
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["name"], "Steve");
        assert_eq!(v["uuid"], "0123456789abcdef0123456789abcdef");
        assert_eq!(v["access_token"], "TOKEN");
        assert_eq!(v["user_type"], "msa");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        // Writing again replaces it.
        write_session(&path, "Alex", "1", "T2").unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains("Alex"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn client_ids_are_uuids() {
        assert!(valid_client_id("12345678-abcd-1234-ABCD-1234567890ab"));
        assert!(!valid_client_id("12345678abcd1234abcd1234567890ab"));
        assert!(!valid_client_id("not-an-id"));
        assert!(!valid_client_id("12345678-abcd-1234-abcd-1234567890az"));
        let dir = std::env::temp_dir().join(format!("hb-save-cid-{}", std::process::id()));
        assert_eq!(
            save_client_id(&dir, "  12345678-abcd-1234-abcd-1234567890ab \n").unwrap(),
            "12345678-abcd-1234-abcd-1234567890ab"
        );
        assert!(save_client_id(&dir, "nope").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn client_id_comes_from_the_file() {
        let dir = std::env::temp_dir().join(format!("hb-cid-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("msa_client_id.txt"), "  abc-123 \nignored\n").unwrap();
        if std::env::var("HORIZON_MSA_CLIENT_ID").is_err()
            && option_env!("HORIZON_MSA_CLIENT_ID").is_none()
        {
            assert_eq!(client_id(&dir).as_deref(), Some("abc-123"));
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
