use super::*;

impl Game<'_> {
    /// `permission_gather_nodes`: the mods declare their typed permission nodes into `nodes` once per
    /// server start, after the operator list is read and before the server starts. Side: server.
    pub(in crate::game) fn native_permission_gather_nodes(
        nodes: &mut dyn mod_api::PermissionNodes,
    ) {
        Self::fire_permission_gather_nodes(nodes);
    }
}
