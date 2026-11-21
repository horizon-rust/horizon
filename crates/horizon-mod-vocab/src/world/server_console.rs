//! What the server's console commands with output and player-limit calls return or refuse with.

/// Why `server_set_max_players` changed nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum PlayerLimitRefusal {
    /// No server is open to players here (a client, or a server nobody can join).
    NotOpen,
    /// The limit is above `i32::MAX`, vanilla's `max-players` is an `int`.
    TooHigh {
        /// The highest limit taken, `i32::MAX`.
        maximum: u32,
    },
}

/// Which of the dedicated server's two ban lists a call reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(horizon_mod_api_macros::Wire)]
pub enum BanListKind {
    /// Name bans, `banned-players.json` (Paper's `BanList.Type.PROFILE`).
    Players,
    /// Address bans, `banned-ips.json` (Paper's `BanList.Type.IP`).
    Ips,
}

/// One entry of a ban list (Paper's `BanEntry`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(horizon_mod_api_macros::Wire)]
pub struct BanRecord {
    /// The banned player's UUID; `None` for an address ban.
    pub uuid: Option<u128>,
    /// The banned player's name; `None` for an address ban.
    pub name: Option<String>,
    /// The banned address; `None` for a name ban.
    pub ip: Option<String>,
    /// When the ban was made, in milliseconds since the Unix epoch.
    pub created_millis: i64,
    /// Who made the ban, as the list's `source` names them (`Server`, `Anticheat`, a player's name).
    pub source: String,
    /// When the ban ends, in milliseconds since the Unix epoch; `None` for a ban that does not end.
    pub expires_millis: Option<i64>,
    /// The reason the ban was given, if any.
    pub reason: Option<String>,
}
