use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A play packet the player `player` (their UUID) sent is about to be handled by the
    /// server (`ServerGamePacketListenerImpl`): every packet of the play phase the hub reads
    /// from the player's input queue, in order. Set `canceled` and the packet is dropped, as if
    /// the client had not sent it. Every packet exposes its protocol type id; custom payloads
    /// also expose their channel and raw bytes. Server-side.
    pub(in crate::game) fn native_serverbound_packet(
        &mut self,
        player: u128,
        packet: &ClientPacket,
        canceled: &mut bool,
    ) {
        self.fire_serverbound_packet(player, packet, canceled);
    }
}
