//! `ban_changed` for native mods: the dedicated server's ban-list changes. A list's watcher queues each change
//! as it happens, on whatever thread changed the list, and the server thread hears them at the next tick.
use super::Game;
use mod_api::{BanListKind, BanRecord};
use std::collections::VecDeque;
use std::sync::Mutex;

/// The most changes kept between two ticks; the oldest are dropped with no notice.
pub(in crate::game) const MAX_PENDING: usize = 4096;

/// Changes made since the last tick, while a mod listens: the list, whether the entry was added, the entry.
pub(in crate::game) static PENDING: Mutex<VecDeque<(BanListKind, bool, BanRecord)>> =
    Mutex::new(VecDeque::new());

/// Queues one ban-list change for `ban_changed`. Nothing is kept while no mod listens.
pub(in crate::game) fn queue(kind: BanListKind, added: bool, record: BanRecord) {
    if !mod_api::__private::listened(mod_api::HookId::BanChanged) {
        return;
    }
    let mut pending = PENDING.lock().unwrap();
    if pending.len() == MAX_PENDING {
        pending.pop_front();
    }
    pending.push_back((kind, added, record));
}

impl Game<'_> {
    /// `ban_changed` for each change queued since the last server tick, in the order they were made.
    pub(super) fn native_ban_changes(&mut self) {
        if !mod_api::__private::listened(mod_api::HookId::BanChanged) {
            PENDING.lock().unwrap().clear();
            return;
        }
        let changes: Vec<(BanListKind, bool, BanRecord)> =
            PENDING.lock().unwrap().drain(..).collect();
        for (kind, added, record) in changes {
            self.native_ban_changed(kind, added, record);
        }
    }
}
