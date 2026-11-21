//! `serverbound_packet`: the game's half of a play packet the hub holds for the mods.
use super::Game;

impl Game<'_> {
    /// The hub holds guest `conn`'s next packet: the mods hear it with the player's state swapped in
    /// and nothing of the hub borrowed, then the hub handles the packet or drops it. The ruling is
    /// always given, also when the player is gone, so the hub never keeps a packet nobody rules on.
    pub(in crate::game) fn lan_packet_heard(&mut self, conn: u32) {
        let heard = self.lan.as_ref().and_then(|host| {
            let (player, packet) = host.hub.heard_packet(conn)?;
            Some((player, super::native_hooks::serverbound_packet_view(packet)))
        });
        let mut canceled = false;
        if let Some((player, view)) = heard {
            self.as_guest(conn, |game| {
                game.native_serverbound_packet(player, &view, &mut canceled);
            });
        }
        if let Some(host) = self.lan.as_mut() {
            host.hub.rule_packet(conn, !canceled);
        }
    }
}
