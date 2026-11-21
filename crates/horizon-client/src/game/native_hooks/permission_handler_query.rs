use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The selected permission handler is asked to answer one node query. Only the mod
    /// `provider`, which provided `handler`, is called; every other mod is skipped before the
    /// call and never sees the player or the contexts. Side: server.
    pub(in crate::game) fn native_permission_handler_query(
        &mut self,
        call: mod_api::__private::PermissionHandlerQueryCall<'_>,
    ) {
        self.fire_permission_handler_query(call);
    }
}
