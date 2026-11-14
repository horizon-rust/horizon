//! The audience of `CommandSourceStack.broadcastToAdmins`
//! (`CommandSourceStack.java:487-497`): the players `PlayerList.isOp` names.
use horizon_client::lan::hub::Hub;

/// `PlayerList.isOp` (`PlayerList.java:587-593`): an `ops.json` entry,
/// whatever its level, or the singleplayer owner while the world allows
/// commands. On an integrated server the owner's level is 4 then and 0
/// otherwise (`IntegratedServer.getProfilePermissions`,
/// `setWorldAllowCommands`), so nobody is an operator without commands.
pub(super) fn is_op(hub: &Hub, uuid: u128) -> bool {
    hub.ops
        .get(&uuid)
        .is_some_and(|&level| level > 0 || hub.singleplayer_owner != Some(uuid))
}

#[cfg(test)]
mod tests;
