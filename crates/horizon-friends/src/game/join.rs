//! Join a Friend: the page the title screen's Friends icon opens (`Place::Title`), drawn as the
//! engine's form. The invite code, the relay a short code goes through (`friendsRelay` in
//! horizon.txt, kept on Join; empty forgets it), and Join, active while the code reads as one.
//! Join asks the game to connect to the code's `friend:` address, which this crate's
//! resolver ([`super::Invites`]) opens.
use super::{Invites, share};
use horizon_platform::{
    hooks::tunnel::Resolver,
    pages::{self, Act, Page, Place, Row},
};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

/// What `horizon-official` registers at the title screen's Friends icon.
pub static PAGE: Join = Join {
    code: Mutex::new(String::new()),
    relay: Mutex::new(String::new()),
};

/// Its rows, in order.
const CODE: usize = 0;
const RELAY: usize = 1;
const JOIN: usize = 2;
/// Invite codes with IPv6 addresses and a relay run past a server address's 128.
const MAX: usize = 256;

/// The page: what its fields hold while it is open.
pub struct Join {
    code: Mutex<String>,
    relay: Mutex<String>,
}

fn lock(text: &Mutex<String>) -> MutexGuard<'_, String> {
    text.lock().unwrap_or_else(|e| e.into_inner())
}

impl Join {
    /// Join: remember the relay in `dir`'s horizon.txt and give the address to connect to, or
    /// None while the code is no invite code.
    fn join(&self, dir: &Path) -> Option<String> {
        let code = lock(&self.code).clone();
        if !Invites.is_code(&code, false) {
            return None;
        }
        let relay = lock(&self.relay).trim().to_owned();
        share::set_preference(dir, "friendsRelay", (!relay.is_empty()).then_some(&relay));
        let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
        Some(format!("{}{code}", crate::ADDRESS_PREFIX))
    }
}

impl Page for Join {
    fn id(&self) -> &'static str {
        "horizon_friends.join"
    }

    fn title(&self) -> &'static str {
        "horizon.multiplayer.join_friend"
    }

    fn place(&self) -> Place {
        Place::Title
    }

    /// An empty code, and the relay the player set before.
    fn opened(&self) {
        lock(&self.code).clear();
        *lock(&self.relay) = share::relay().unwrap_or_default();
    }

    fn rows(&self, rows: &mut Vec<Row>) {
        let code = lock(&self.code).clone();
        rows.push(Row::field("horizon.multiplayer.invite_code", code.clone()).max(MAX));
        rows.push(Row::field("horizon.multiplayer.relay", lock(&self.relay).clone()).max(MAX));
        rows.push(Row::button("horizon.multiplayer.join").active(Invites.is_code(&code, false)));
    }

    /// Enter in a field does nothing, as on the screen this page replaces.
    fn act(&self, row: usize, act: Act) {
        match (row, act) {
            (CODE, Act::Edit(text)) => *lock(&self.code) = text,
            (RELAY, Act::Edit(text)) => *lock(&self.relay) = text,
            (JOIN, Act::Press) => {
                let dir = std::env::current_dir().unwrap_or_default();
                if let Some(address) = self.join(&dir) {
                    pages::connect(address);
                }
            }
            _ => {}
        }
    }

    fn back(&self) -> &'static str {
        "gui.cancel"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code::Code;

    fn page() -> Join {
        Join {
            code: Mutex::new(String::new()),
            relay: Mutex::new(String::new()),
        }
    }

    fn join_active(page: &Join) -> bool {
        let mut rows = Vec::new();
        page.rows(&mut rows);
        matches!(rows[JOIN], Row::Button { active: true, .. })
    }

    #[test]
    fn join_takes_an_invite_code_and_keeps_the_relay() {
        let dir = std::env::temp_dir().join(format!("horizon-friends-join-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("horizon.txt"),
            "friendsRelay:old.example:1\nother:1\n",
        )
        .unwrap();
        let page = page();
        // A nameplate alone is no code.
        page.act(CODE, Act::Edit("K7QM".into()));
        assert!(!join_active(&page));
        assert_eq!(page.join(&dir), None);
        let code = Code {
            nameplate: "K7QM".into(),
            secret: "3XPD".into(),
        };
        page.act(CODE, Act::Edit(format!(" {code} ")));
        page.act(RELAY, Act::Edit(" relay.example:443 ".into()));
        assert!(join_active(&page));
        assert_eq!(page.join(&dir), Some(format!("friend:{code}")));
        let saved = std::fs::read_to_string(dir.join("horizon.txt")).unwrap();
        assert_eq!(saved, "other:1\nfriendsRelay:relay.example:443\n");
        // An empty relay forgets it.
        page.act(RELAY, Act::Edit(String::new()));
        assert!(page.join(&dir).is_some());
        assert_eq!(
            std::fs::read_to_string(dir.join("horizon.txt")).unwrap(),
            "other:1\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
