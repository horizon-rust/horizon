//! `server_ban_list`: the entries of the dedicated server's ban lists, read live.
use super::Game;
use horizon_server_files::user_lists::{BanEntry, NameAndId};
use mod_api::{BanListKind, BanRecord};

impl Game<'_> {
    /// The entries of one ban list as its file holds them. Empty where no server list is loaded.
    pub(super) fn server_ban_list(&self, kind: BanListKind) -> Vec<BanRecord> {
        let Some(lists) = self.server_lists.as_ref() else {
            return Vec::new();
        };
        let lists = lists.lock().unwrap();
        let out: Vec<BanRecord> = match kind {
            BanListKind::Players => lists
                .bans
                .entries()
                .into_iter()
                .map(player_record)
                .collect(),
            BanListKind::Ips => lists.ip_bans.entries().into_iter().map(ip_record).collect(),
        };
        out
    }
}

/// A name ban as the mod API reports it.
pub(in crate::game) fn player_record(ban: &BanEntry<NameAndId>) -> BanRecord {
    BanRecord {
        uuid: Some(ban.user.id),
        name: Some(ban.user.name.clone()),
        ip: None,
        created_millis: ban.created,
        source: ban.source.clone(),
        expires_millis: ban.expires,
        reason: ban.reason.clone(),
    }
}

/// An address ban as the mod API reports it.
pub(in crate::game) fn ip_record(ban: &BanEntry<String>) -> BanRecord {
    BanRecord {
        uuid: None,
        name: None,
        ip: Some(ban.user.clone()),
        created_millis: ban.created,
        source: ban.source.clone(),
        expires_millis: ban.expires,
        reason: ban.reason.clone(),
    }
}
