//! Vanilla's friends list: the `player` service's `getFriends`, `updateFriends`,
//! `updateAttributes` and `updatePresence` (`MinecraftServicesFriendsService`), and the
//! state `PlayerSocialManager`/`RemoteFriendListUpdateHandler` keep for the screens.
//!
//! The service is reached through the discovery document with the account's bearer
//! token, on a worker; the screens read the latest answer and never wait. With no
//! session, or when the account may not use friends (`friendsEnabled` false), no
//! request is made and the lists stay empty.
use crate::auth::{self, AuthSession};
use serde_json::Value;
use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// `FriendsService.ResultCode`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultCode {
    Success,
    Error,
    ServiceNotAvailable,
    TooManyRequests,
    Forbidden,
    UpgradeNeeded,
    ConnectionIssue,
    TemporaryUnavailable,
    UnknownProfile,
    Unauthorized,
    GenericError,
}

/// `FriendDto`: one friend or one pending request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Friend {
    pub id: u128,
    pub name: String,
}

/// `FriendData`: the three lists the service answers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FriendData {
    pub friends: Vec<Friend>,
    pub incoming: Vec<Friend>,
    pub outgoing: Vec<Friend>,
}

/// `PresenceStatus`, as the service stores it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresenceStatus {
    Online,
    PlayingOffline,
    PlayingRealms,
    PlayingServer,
    PlayingHostedServer,
    Offline,
}
impl PresenceStatus {
    /// `PresenceStatus.name()`: the `updatePresence` body's value.
    fn wire(self) -> &'static str {
        match self {
            Self::Online => "ONLINE",
            Self::PlayingOffline => "PLAYING_OFFLINE",
            Self::PlayingRealms => "PLAYING_REALMS",
            Self::PlayingServer => "PLAYING_SERVER",
            Self::PlayingHostedServer => "PLAYING_HOSTED_SERVER",
            Self::Offline => "OFFLINE",
        }
    }
    /// `gui.friends.presence.status.<name>`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Online => "gui.friends.presence.status.online",
            Self::PlayingOffline => "gui.friends.presence.status.playing_offline",
            Self::PlayingRealms => "gui.friends.presence.status.playing_realms",
            Self::PlayingServer => "gui.friends.presence.status.playing_server",
            Self::PlayingHostedServer => "gui.friends.presence.status.playing_hosted_server",
            Self::Offline => "gui.friends.presence.status.offline",
        }
    }
    /// `FriendEntry.presenceStatusSortOrder`: playing friends first, offline last.
    pub fn sort_order(self) -> u8 {
        match self {
            Self::PlayingHostedServer => 0,
            Self::PlayingServer => 1,
            Self::PlayingRealms => 2,
            Self::PlayingOffline => 3,
            Self::Online => 4,
            Self::Offline => 5,
        }
    }
}

/// `PresenceStatusDto` as the friends screen reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Presence {
    pub id: u128,
    pub status: PresenceStatus,
}

/// `RemoteFriendListUpdateHandler.State`: what the overlay says about the lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Loading,
    UpgradeNeeded,
    ConnectionIssue,
    UserMayLackActiveProfile,
    Unauthorized,
    TemporaryUnavailable,
    GenericError,
    Success,
}
impl State {
    fn of(code: ResultCode) -> Self {
        match code {
            ResultCode::Success => Self::Success,
            ResultCode::UpgradeNeeded => Self::UpgradeNeeded,
            ResultCode::ConnectionIssue => Self::ConnectionIssue,
            ResultCode::UnknownProfile => Self::UserMayLackActiveProfile,
            ResultCode::Unauthorized => Self::Unauthorized,
            ResultCode::TemporaryUnavailable
            | ResultCode::Forbidden
            | ResultCode::ServiceNotAvailable
            | ResultCode::TooManyRequests => Self::TemporaryUnavailable,
            ResultCode::Error | ResultCode::GenericError => Self::GenericError,
        }
    }
}

/// `Options.sharePresence` (`PresenceSharing`): how much of the player the service sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharePresence {
    None,
    Limited,
    All,
}

impl SharePresence {
    /// `options.txt`'s `sharePresence` (0 none, 1 limited, 2 all).
    pub fn of(setting: u8) -> Self {
        match setting {
            0 => Self::None,
            1 => Self::Limited,
            _ => Self::All,
        }
    }
}

/// The service's end state: lists, presence, flags and the poll bookkeeping.
struct Inner {
    /// `friendsEnabled`/`allowFriendRequests`: `friendsPreferences` of the user attributes.
    enabled: bool,
    allow_requests: bool,
    data: FriendData,
    state: State,
    presence: Vec<Presence>,
    /// When the last lists answer arrived, and the last presence post.
    lists_at: Option<Instant>,
    presence_at: Option<Instant>,
    /// The overlay is open: `RemoteFriendListUpdateHandler`'s foreground interval.
    foreground: bool,
    share: SharePresence,
    /// `PresenceHandler.getPresenceStatus` of an all-sharing player.
    playing: PresenceStatus,
    /// `Retry-After` from the service, when it answered one (seconds).
    retry_after: Option<Duration>,
    /// A refresh is running (one at a time, as `requestPending`).
    refreshing: bool,
    started: bool,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            enabled: false,
            allow_requests: false,
            data: FriendData::default(),
            state: State::Loading,
            presence: Vec::new(),
            lists_at: None,
            presence_at: None,
            foreground: false,
            share: SharePresence::All,
            playing: PresenceStatus::Online,
            retry_after: None,
            refreshing: false,
            started: false,
        }
    }
}

fn inner() -> &'static Arc<Mutex<Inner>> {
    static INNER: OnceLock<Arc<Mutex<Inner>>> = OnceLock::new();
    INNER.get_or_init(|| Arc::new(Mutex::new(Inner::default())))
}

fn lock() -> std::sync::MutexGuard<'static, Inner> {
    inner().lock().unwrap_or_else(|e| e.into_inner())
}

/// `PlayerSocialManager`'s constructor: the account's `friendsPreferences` decide the
/// flags; enabled starts the updater (`RemoteFriendListUpdateHandler.start`).
pub fn init(enabled: bool, allow_requests: bool) {
    let first = {
        let mut inner = lock();
        inner.enabled = enabled;
        inner.allow_requests = allow_requests;
        if inner.started {
            return;
        }
        inner.started = true;
        enabled
    };
    // The poller runs whether or not the list starts on: enabling it later
    // (`updateAttributes`) starts the requests from the same worker.
    spawn_poller();
    if first {
        // The first answer is as quick as the service allows.
        refresh();
    }
}

/// `Minecraft.friendsEnabled`: an account that may use the friends list. Offline play
/// (no session) has none.
pub fn available() -> bool {
    auth::current().filter(|s| !s.expired()).is_some()
}

/// [`update_settings`] on a worker: the rows read the flags when the service answers.
pub fn update_settings_async(friends_enabled: bool, allow_requests: bool) {
    let _ = std::thread::Builder::new()
        .name("Friends settings".into())
        .spawn(move || {
            update_settings(friends_enabled, allow_requests);
        });
}

/// Whether the account's friends list may be used (`Minecraft.friendsEnabled`).
pub fn is_enabled() -> bool {
    lock().enabled
}

/// `PlayerSocialManager.isAllowFriendRequests`.
pub fn allow_requests() -> bool {
    lock().allow_requests
}

/// The latest lists.
pub fn data() -> FriendData {
    lock().data.clone()
}

/// The latest update's state.
pub fn state() -> State {
    lock().state
}

/// `PlayerSocialManager.getPresenceHandler().getLatestPresence()`, by profile.
pub fn presence_of(id: u128) -> Option<PresenceStatus> {
    lock()
        .presence
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.status)
}

/// All known presence, for the overlay's status lines.
pub fn presence() -> Vec<Presence> {
    lock().presence.clone()
}

/// `FriendsButton.refreshIncomingRequestCount`.
pub fn incoming_count() -> usize {
    let inner = lock();
    if inner.enabled {
        inner.data.incoming.len()
    } else {
        0
    }
}

/// `RemoteFriendListUpdateHandler`'s foreground interval: the overlay is open.
pub fn set_foreground(open: bool) {
    lock().foreground = open;
}

/// `Options.sharePresence`: what presence the poller posts.
pub fn set_share_presence(share: SharePresence) {
    lock().share = share;
}

/// `PresenceHandler.getPresenceStatus`: what an all-sharing player is doing.
pub fn set_playing(status: PresenceStatus) {
    lock().playing = status;
}

/// `FriendsService.updateFriendSettings`: the account's flags, server-side.
pub fn update_settings(friends_enabled: bool, allow_requests: bool) -> ResultCode {
    let Some(session) = session() else {
        return ResultCode::Unauthorized;
    };
    let body = serde_json::json!({
        "friendsPreferences": {
            "friends": toggle(friends_enabled),
            "acceptInvites": toggle(allow_requests),
        }
    });
    let code = match post_json(&session, "updateAttributes", &body.to_string()) {
        Ok(Some(_)) => ResultCode::Success,
        Ok(None) => ResultCode::Success,
        Err(code) => code,
    };
    if code == ResultCode::Success {
        {
            let mut inner = lock();
            inner.enabled = friends_enabled;
            inner.allow_requests = allow_requests;
            if !friends_enabled {
                inner.data = FriendData::default();
                inner.state = State::Loading;
                inner.lists_at = None;
            }
        }
        // `updateFriendSettings`' answer is followed by the lists' first read
        // (`RemoteFriendListUpdateHandler.start`).
        if friends_enabled {
            refresh();
        }
    }
    code
}

fn toggle(on: bool) -> &'static str {
    if on { "ENABLED" } else { "DISABLED" }
}

/// `PlayerSocialManager.sendFriendRequest(name)`.
pub fn send_request(name: &str) -> ResultCode {
    action(serde_json::json!({"name": name, "updateType": "ADD"}))
}

/// `PlayerSocialManager.removeFriend`/`declineIncomingFriendRequest`/
/// `revokeOutgoingFriendRequest` (`updateType` REMOVE).
pub fn remove(id: u128) -> ResultCode {
    action(serde_json::json!({"profileId": uuid(id), "updateType": "REMOVE"}))
}

/// `PlayerSocialManager.acceptIncomingFriendRequest`: ADD by profile id.
pub fn accept(id: u128) -> ResultCode {
    action(serde_json::json!({"profileId": uuid(id), "updateType": "ADD"}))
}

/// `updateFriends`: the answer already is the new lists, as authlib caches it.
fn action(body: Value) -> ResultCode {
    let Some(session) = session() else {
        return ResultCode::Unauthorized;
    };
    match put_json(&session, "updateFriends", &body.to_string()) {
        Ok(Some(data)) => {
            let mut inner = lock();
            inner.data = data;
            inner.state = State::Success;
            inner.lists_at = Some(Instant::now());
            ResultCode::Success
        }
        Ok(None) => ResultCode::Success,
        Err(code) => code,
    }
}

/// `MinecraftServicesFriendsService.getFriendData`: one update, blocking.
pub fn refresh() -> State {
    {
        let inner = lock();
        if !inner.enabled || inner.refreshing {
            return inner.state;
        }
    }
    lock().refreshing = true;
    let Some(session) = session() else {
        let mut inner = lock();
        inner.refreshing = false;
        inner.state = State::Unauthorized;
        return inner.state;
    };
    let result = get_friends(&session);
    let mut inner = lock();
    inner.refreshing = false;
    inner.lists_at = Some(Instant::now());
    match result {
        Ok(data) => {
            inner.data = data;
            inner.state = State::Success;
        }
        Err((code, retry)) => {
            inner.state = State::of(code);
            if let Some(retry) = retry {
                inner.retry_after = Some(retry);
            }
        }
    }
    inner.state
}

/// `MinecraftServicesFriendsService.updateFriendSettings`' answer drives `init`.
fn session() -> Option<Arc<AuthSession>> {
    auth::current().filter(|s| !s.expired())
}

fn uuid(id: u128) -> String {
    let hex = format!("{id:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn url(name: &str) -> Option<String> {
    let doc = auth::discovery().ok()?;
    auth::endpoint(&doc, "player", name)
}

fn get_friends(session: &AuthSession) -> Result<FriendData, (ResultCode, Option<Duration>)> {
    let url = url("getFriends").ok_or((ResultCode::ServiceNotAvailable, None))?;
    let response = auth::agent()
        .get(&url)
        .set(
            "Authorization",
            &format!("Bearer {}", session.access_token()),
        )
        .call()
        .map_err(|e| (http_error(e), None))?;
    let body = response
        .into_string()
        .map_err(|_| (ResultCode::GenericError, None))?;
    parse_friends(&body).ok_or((ResultCode::GenericError, None))
}

fn put_json(
    session: &AuthSession,
    endpoint: &str,
    body: &str,
) -> Result<Option<FriendData>, ResultCode> {
    let url = url(endpoint).ok_or(ResultCode::ServiceNotAvailable)?;
    let response = auth::agent()
        .put(&url)
        .set(
            "Authorization",
            &format!("Bearer {}", session.access_token()),
        )
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(body)
        .map_err(http_error)?;
    let text = response
        .into_string()
        .map_err(|_| ResultCode::GenericError)?;
    if text.is_empty() {
        return Ok(None);
    }
    parse_friends(&text)
        .map(Some)
        .ok_or(ResultCode::GenericError)
}

fn post_json(
    session: &AuthSession,
    endpoint: &str,
    body: &str,
) -> Result<Option<String>, ResultCode> {
    let url = url(endpoint).ok_or(ResultCode::ServiceNotAvailable)?;
    let response = auth::agent()
        .post(&url)
        .set(
            "Authorization",
            &format!("Bearer {}", session.access_token()),
        )
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(body)
        .map_err(http_error)?;
    response
        .into_string()
        .map(Some)
        .map_err(|_| ResultCode::GenericError)
}

/// `MinecraftServicesFriendsService.handleHttpError`, plus the transport failure.
fn http_error(error: ureq::Error) -> ResultCode {
    match error {
        ureq::Error::Status(status, _) => match status {
            429 => ResultCode::TooManyRequests,
            500.. => ResultCode::ServiceNotAvailable,
            403 => ResultCode::Forbidden,
            400 => ResultCode::UnknownProfile,
            401 => ResultCode::Unauthorized,
            _ => ResultCode::Error,
        },
        ureq::Error::Transport(_) => ResultCode::ConnectionIssue,
    }
}

/// `FriendsListResponse` (the `getFriends`/`updateFriends` body).
pub fn parse_friends(body: &str) -> Option<FriendData> {
    let v: Value = serde_json::from_str(body).ok()?;
    Some(FriendData {
        friends: parse_friend_list(v.get("friends")),
        incoming: parse_friend_list(v.get("incomingRequests")),
        outgoing: parse_friend_list(v.get("outgoingRequests")),
    })
}

fn parse_friend_list(value: Option<&Value>) -> Vec<Friend> {
    value
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|friend| {
            Some(Friend {
                id: auth::parse_uuid(friend.get("profileId")?.as_str()?)?,
                name: friend.get("name")?.as_str()?.to_owned(),
            })
        })
        .collect()
}

/// `PresenceResponse`.
pub fn parse_presence(body: &str) -> Option<Vec<Presence>> {
    let v: Value = serde_json::from_str(body).ok()?;
    Some(
        v.get("presence")?
            .as_array()?
            .iter()
            .filter_map(|entry| {
                let status = entry.get("status")?.as_str()?;
                let status = match status {
                    "ONLINE" => PresenceStatus::Online,
                    "PLAYING_OFFLINE" => PresenceStatus::PlayingOffline,
                    "PLAYING_REALMS" => PresenceStatus::PlayingRealms,
                    "PLAYING_SERVER" => PresenceStatus::PlayingServer,
                    "PLAYING_HOSTED_SERVER" => PresenceStatus::PlayingHostedServer,
                    _ => PresenceStatus::Offline,
                };
                Some(Presence {
                    id: auth::parse_uuid(entry.get("profileId")?.as_str()?)?,
                    status,
                })
            })
            .collect(),
    )
}

/// `RemoteFriendListUpdateHandler.start`: a worker that keeps the lists and the
/// presence current. One request each at a time; the interval is the service's
/// `Retry-After` or a minute in the foreground and five in the background.
fn spawn_poller() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("Friends List".into())
        .spawn(|| {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let (refresh_lists, post) = {
                    let inner = lock();
                    if !inner.enabled {
                        (false, false)
                    } else {
                        let interval = inner.retry_after.unwrap_or(Duration::from_secs(60))
                            * if inner.foreground { 1 } else { 5 };
                        let due = inner.lists_at.is_none_or(|at| at.elapsed() >= interval);
                        let presence_interval =
                            Duration::from_secs(60) * if inner.foreground { 1 } else { 5 };
                        let presence_due = inner
                            .presence_at
                            .is_none_or(|at| at.elapsed() >= presence_interval);
                        (due, presence_due)
                    }
                };
                if refresh_lists {
                    refresh();
                }
                if post {
                    post_presence();
                }
            }
        });
}

/// `PresenceHandler.updatePresence`: the public status for `sharePresence`.
fn post_presence() {
    let (share, playing) = {
        let inner = lock();
        (inner.share, inner.playing)
    };
    let status = match share {
        SharePresence::None => PresenceStatus::Offline,
        SharePresence::Limited => PresenceStatus::Online,
        SharePresence::All => playing,
    };
    let Some(session) = session() else { return };
    let url = url("updatePresence");
    let mut inner = lock();
    inner.presence_at = Some(Instant::now());
    drop(inner);
    let Some(url) = url else { return };
    let body = serde_json::json!({"status": status.wire()}).to_string();
    let Ok(response) = auth::agent()
        .post(&url)
        .set(
            "Authorization",
            &format!("Bearer {}", session.access_token()),
        )
        .set("Content-Type", "application/json; charset=utf-8")
        .send_string(&body)
    else {
        return;
    };
    if let Ok(text) = response.into_string()
        && let Some(presence) = parse_presence(&text)
    {
        lock().presence = presence;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_friends_answer_reads_its_three_lists() {
        let data = parse_friends(
            r#"{"friends":[{"profileId":"0123456789abcdef0123456789abcdef","name":"Alex"}],
                "incomingRequests":[],"outgoingRequests":[{"profileId":"fedcba9876543210fedcba9876543210","name":"Steve"}]}"#,
        )
        .unwrap();
        assert_eq!(
            data.friends,
            vec![Friend {
                id: 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef,
                name: "Alex".into()
            }]
        );
        assert!(data.incoming.is_empty());
        assert_eq!(data.outgoing[0].name, "Steve");
    }

    #[test]
    fn a_presence_answer_reads_each_status() {
        let presence = parse_presence(
            r#"{"presence":[
                {"profileId":"0123456789abcdef0123456789abcdef","pmid":"00000000000000000000000000000000","status":"PLAYING_REALMS","lastUpdated":"2026-11-08T12:00:00Z"},
                {"profileId":"fedcba9876543210fedcba9876543210","pmid":"00000000000000000000000000000000","status":"OFFLINE","lastUpdated":"2026-11-08T12:00:00Z"}]}"#,
        )
        .unwrap();
        assert_eq!(presence[0].status, PresenceStatus::PlayingRealms);
        assert_eq!(presence[1].status, PresenceStatus::Offline);
    }

    #[test]
    fn result_codes_map_to_the_overlays_states() {
        assert_eq!(State::of(ResultCode::Success), State::Success);
        assert_eq!(
            State::of(ResultCode::TooManyRequests),
            State::TemporaryUnavailable
        );
        assert_eq!(
            State::of(ResultCode::UnknownProfile),
            State::UserMayLackActiveProfile
        );
        assert_eq!(State::of(ResultCode::UpgradeNeeded), State::UpgradeNeeded);
        assert_eq!(
            State::of(ResultCode::ConnectionIssue),
            State::ConnectionIssue
        );
    }

    #[test]
    fn a_status_sorts_playing_friends_first() {
        assert!(
            PresenceStatus::PlayingHostedServer.sort_order() < PresenceStatus::Online.sort_order()
        );
        assert!(PresenceStatus::Online.sort_order() < PresenceStatus::Offline.sort_order());
    }
}
