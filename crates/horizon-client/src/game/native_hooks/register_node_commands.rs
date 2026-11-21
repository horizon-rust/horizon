use super::*;

impl Game<'_> {
    /// `register_node_commands`: the mods push the server commands whose permission is a Boolean
    /// node, whenever `native_register_commands` is asked. Side: server.
    pub(in crate::game) fn native_register_node_commands(
        &mut self,
        commands: &mut Vec<mod_api::NodeCommand>,
    ) {
        self.fire_register_node_commands(commands);
    }
}
