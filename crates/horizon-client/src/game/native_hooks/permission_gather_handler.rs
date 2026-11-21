use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Once per logical server start, after the operator levels load and before the server
    /// starts: each mod names the permission handlers it provides, in the order the mods run.
    /// Side: server.
    pub(in crate::game) fn native_permission_gather_handler(
        &mut self,
        handlers: &mut Vec<mod_api::Answered<String>>,
    ) {
        self.fire_permission_gather_handler(handlers);
    }
}
