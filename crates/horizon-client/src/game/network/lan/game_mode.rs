//! The server's forced game mode: `MinecraftServer.getForcedGameType` and
//! `enforceGameTypeForPlayers`, `IntegratedServer`'s overrides of them, and the
//! `/defaultgamemode`, world-options and `/publish` paths that apply them.
use super::{Game, LanHost};
use horizon_client::{lan::hub::HubConfig, world::World};
use horizon_world::block_collision::BlockCollisionRegistry;

/// `MinecraftServer.getForcedGameType`: the mode the server puts the players
/// online in, `None` when it forces none. An integrated server forces its
/// world's mode only once it is published, and never in a hardcore world
/// (`IntegratedServer.getForcedGameType`).
pub(super) fn forced_game_type(config: &HubConfig, published: bool, hardcore: bool) -> Option<i32> {
    if !config.force_game_mode || (config.integrated && !(published && !hardcore)) {
        return None;
    }
    Some(config.default_game_mode)
}

/// With the game mode forced, the guests online take `mode` (setting a mode
/// does nothing when it is already theirs). A world open to LAN is an
/// integrated server, which leaves out the players who have game master
/// permission; a dedicated server does not
/// (`MinecraftServer.enforceGameTypeForPlayers` and `IntegratedServer`'s
/// override). The guest `acting` (whose turn it is, `acting_mode` theirs) is
/// left to the caller: their state is the game's until the turn ends. Whether
/// it switches them, and how many it switched.
fn enforce_guest_game_mode(
    host: &mut LanHost,
    mode: i32,
    acting: Option<(u32, i32)>,
    world: &World,
    collision: &BlockCollisionRegistry<'_>,
) -> (bool, i32) {
    if !host.hub.config.force_game_mode {
        return (false, 0);
    }
    let spares_operators = host.hub.config.integrated;
    let stale: Vec<u32> = host
        .hub
        .guests
        .iter()
        .filter(|g| {
            let current = match acting {
                Some((conn, own)) if conn == g.conn => own,
                _ => g.game_mode,
            };
            current != mode && !(spares_operators && g.permission >= 2)
        })
        .map(|g| g.conn)
        .collect();
    let mut acting_switched = false;
    for &conn in &stale {
        if acting.is_some_and(|(c, _)| c == conn) {
            acting_switched = true;
        } else {
            host.hub.set_game_mode(conn, mode, world, Some(collision));
        }
    }
    (acting_switched, stale.len() as i32)
}

impl Game<'_> {
    /// The world's default game mode (`MinecraftServer.setDefaultGameType`)
    /// and, with the mode forced, the players it switches
    /// (`DefaultGameModeCommands.setMode`). How many it switched.
    pub(in crate::game) fn set_default_game_mode(&mut self, mode: i32) -> i32 {
        self.level.game_mode = mode;
        self.level.creative = mode == 1;
        if let Some(host) = self.lan.as_mut() {
            host.hub.config.default_game_mode = mode;
        }
        self.enforce_game_type_for_players(self.forced_game_type())
    }

    /// `MinecraftServer.getForcedGameType`: the mode the server puts a player
    /// in, when it forces one.
    pub(in crate::game) fn forced_game_type(&self) -> Option<i32> {
        let host = self.lan.as_ref()?;
        forced_game_type(
            &host.hub.config,
            self.published_port().is_some(),
            self.level.hardcore,
        )
    }

    /// `MinecraftServer.enforceGameTypeForPlayers`: the players online take
    /// `mode`, and how many changed.
    pub(in crate::game) fn enforce_game_type_for_players(&mut self, mode: Option<i32>) -> i32 {
        let Some(mode) = mode else {
            return 0;
        };
        self.enforce_guest_game_mode(mode)
    }

    /// `IntegratedServer.updateGameModeBasedOnPermission`: with the mode
    /// forced, the players online who may not change their own take the
    /// world's mode.
    pub(in crate::game) fn update_game_mode_based_on_permission(&mut self) -> i32 {
        let Some(mode) = self.lan.as_ref().map(|h| h.hub.config.default_game_mode) else {
            return 0;
        };
        self.enforce_guest_game_mode(mode)
    }

    /// `enforce_guest_game_mode`, the guest whose turn it is included: a
    /// command's player is one of the players online, and changes mode as
    /// `/gamemode` changes theirs (`ServerPlayer.setGameMode`), so that the
    /// end of the turn keeps it. How many it switched.
    fn enforce_guest_game_mode(&mut self, mode: i32) -> i32 {
        let acting = self.lan_acting.map(|conn| (conn, self.mode_id()));
        let Some(host) = self.lan.as_mut() else {
            return 0;
        };
        let (acting_switched, count) =
            enforce_guest_game_mode(host, mode, acting, &self.world, self.collision);
        if acting_switched {
            self.set_mode(super::super::game_mode::mode_from_id(mode));
        }
        count
    }

    /// `IntegratedServer.setForceGameMode`: the players take the mode when it
    /// is turned on.
    pub(in crate::game) fn set_force_game_mode(&mut self, force: bool) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        host.settings.force_game_mode = force;
        host.hub.config.force_game_mode = force;
        let mode = host.hub.config.default_game_mode;
        self.enforce_guest_game_mode(mode);
    }
}
