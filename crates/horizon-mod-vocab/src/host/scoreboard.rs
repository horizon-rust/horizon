//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The server host's methods of the `scoreboard` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait ScoreboardServerHost {
    /// Side: server.
    /// The team `name` (`Scoreboard.getPlayerTeam`), or None when there is none, for a name that
    /// is empty or has a space, and on a client. Teams are kept with the world's scoreboard,
    /// which the server saves.
    #[access(WorldOnly)]
    fn team(&self, name: &str) -> Option<TeamRef>;
    /// Side: server.
    /// `Scoreboard.addPlayerTeam`, as `/team add`: a new team with no colour and no members.
    /// `NameTaken` when a team of that name exists; `Unavailable` on a client and for a name
    /// that is empty, has a space, or one the saved scoreboard could not keep (its modified
    /// UTF-8 form is over 65535 bytes).
    #[access(WorldOnly)]
    fn add_team(&mut self, name: &str) -> Result<TeamRef, TeamError>;
    /// Side: server.
    /// `PlayerTeam.setColor`, as `/team modify <team> color`: the team's members' names take
    /// the colour in every client from the next tick's scoreboard sync, and None (`reset`)
    /// takes it away. True when the colour changed; false for a stale team, on a client and
    /// when the team already has that colour.
    #[access(WorldOnly)]
    fn team_set_color(&mut self, team: TeamRef, color: Option<TeamColor>) -> bool;
    /// Side: server.
    /// `PlayerTeam.setPlayerPrefix`, as `/team modify <team> prefix`: the text shown before
    /// each member's name in chat, the Tab list and above their heads, in every client from
    /// the next tick's scoreboard sync (an empty text takes it away). Ok(true) when the prefix
    /// changed; Ok(false) for a stale team, on a client and when the team has that prefix.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn team_set_prefix(&mut self, team: TeamRef, prefix: Text) -> Result<bool, TextFormError>;
    /// Side: server.
    /// `PlayerTeam.setPlayerSuffix`, as `/team modify <team> suffix`: the same as
    /// `team_set_prefix`, after each member's name.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn team_set_suffix(&mut self, team: TeamRef, suffix: Text) -> Result<bool, TextFormError>;
    /// Side: server.
    /// `PlayerTeam.setNameTagVisibility`, as `/team modify <team> nametagVisibility`: who sees
    /// the members' name tags. True when it changed; false for a stale team, on a client and
    /// when the team has that value.
    #[access(WorldOnly)]
    fn team_set_name_tag_visibility(&mut self, team: TeamRef, visibility: TeamVisibility) -> bool;
    /// Side: server.
    /// `PlayerTeam.setCollisionRule`, as `/team modify <team> collisionRule`: how the members
    /// collide. True when it changed; false for a stale team, on a client and when the team
    /// has that rule.
    #[access(WorldOnly)]
    fn team_set_collision_rule(&mut self, team: TeamRef, rule: TeamCollisionRule) -> bool;
    /// Side: server.
    /// `PlayerTeam.setAllowFriendlyFire`, as `/team modify <team> friendlyFire`: whether
    /// members can hurt each other (a new team allows it). True when it changed; false for a
    /// stale team, on a client and when the team has that value.
    #[access(WorldOnly)]
    fn team_set_friendly_fire(&mut self, team: TeamRef, allowed: bool) -> bool;
    /// Side: server.
    /// `Scoreboard.addPlayerToTeam`, as `/team join`: `holder` leaves the team it is on and
    /// joins `team`, in every client from the next tick's sync. Ok(false) when it is on that
    /// team already. `Stale` for a team that is gone, `UnknownHolder` for a holder the
    /// scoreboard cannot key (an empty name, one the saved scoreboard could not keep, a stale
    /// player or entity), `Unavailable` on a client.
    #[access(WorldOnly)]
    fn team_join(&mut self, team: TeamRef, holder: ScoreHolder) -> Result<bool, TeamError>;
    /// Side: server.
    /// `Scoreboard.removePlayerFromTeam`, as `/team leave`: `holder` leaves `team`. False for
    /// a stale team, a holder that is not on it, and on a client.
    #[access(WorldOnly)]
    fn team_leave(&mut self, team: TeamRef, holder: ScoreHolder) -> bool;
    /// Side: server.
    /// `Scoreboard.removePlayerTeam`, as `/team remove`: the team is gone and its members have
    /// no team. False for a stale team and on a client.
    #[access(WorldOnly)]
    fn team_remove(&mut self, team: TeamRef) -> bool;
    /// Side: server.
    /// The name of the team the entity is on (`Entity.getTeam`, Paper's
    /// `Scoreboard#getEntryTeam`), None when it is on no team. The scoreboard keys an entity
    /// by its UUID, so this reads the entry `team_join` writes for it. Reads only: a stale
    /// entity, a client, or an entity whose UUID the server cannot give gives None.
    #[access(WorldOnly)]
    fn entity_team(&mut self, entity: EntityRef) -> Option<String>;
    /// Side: server.
    /// The score a holder has on an objective (`Scoreboard.getOrCreateScore`'s value, Paper's
    /// `Score#getScore` through `Objective#getScore`). The holder is the scoreboard's key: a
    /// player's name, an entity's UUID as text, or a name as written (`/scoreboard players`).
    /// None when the holder has no score on that objective, the objective does not exist, or
    /// this is a client, whose scoreboard the server keeps.
    #[access(WorldOnly)]
    fn scoreboard_score(&self, holder: &str, objective: &str) -> Option<i32>;
}
