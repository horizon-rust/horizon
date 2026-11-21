//! A play packet the mods may veto (`serverbound_packet`): held by the hub, ruled on by the game.
//!
//! With a listener ([`Hub::packets_listened`]) the hub does not handle a packet the moment it reads
//! it from a player's input queue. It keeps the packet and its input credit in one slot, tells the
//! game ([`HostEvent::PacketHeard`]) and pauses the turn the way a raw payload does, so the player's
//! later packets stay in order behind it. The game fires the hook with nothing of the hub borrowed
//! and calls [`Hub::rule_packet`]: the packet goes back to the head of the queue and is handled
//! without being asked again, or it is dropped with its credit. With no listener nothing is held.

use super::{HostEvent, Hub, InboxItem};
use crate::{lan::NetEvent, net::ingress::ReceiveLease};
use horizon_protocol::packets::play::Serverbound as S;

type Credit = Option<ReceiveLease<NetEvent>>;

/// The packet the game is ruling on, and whose it is.
pub(super) struct HeardPacket {
    conn: u32,
    packet: S,
    credit: Credit,
}

impl Hub {
    /// Hands `packet` back to be handled, or keeps it for the mods and pauses the turn (None): the
    /// credits read so far go to `events` behind [`HostEvent::PacketHeard`], the packet's own stays
    /// with it. A packet the game just passed is handed back once, unasked.
    pub(super) fn hear_or_keep(
        &mut self,
        conn: u32,
        packet: S,
        credit: Credit,
        credits: &mut Vec<ReceiveLease<NetEvent>>,
        events: &mut Vec<HostEvent>,
    ) -> Option<(S, Credit)> {
        if !self.packets_listened || self.heard_pass.take() == Some(conn) {
            return Some((packet, credit));
        }
        self.heard = Some(HeardPacket {
            conn,
            packet,
            credit,
        });
        events.push(HostEvent::PacketHeard(conn));
        events.push(HostEvent::InputCredits(std::mem::take(credits)));
        None
    }

    /// What the hook is told of the packet held for guest `conn`: the sender and the packet.
    pub fn heard_packet(&self, conn: u32) -> Option<(u128, &S)> {
        let held = self.heard.as_ref().filter(|held| held.conn == conn)?;
        Some((self.guest(conn)?.uuid, &held.packet))
    }

    /// The mods ruled on the packet held for guest `conn`: `pass` puts it back at the head of the
    /// queue, to be handled next; otherwise it is dropped, its credit with it. A guest who left
    /// meanwhile loses the packet either way. Nothing happens when no packet is held for `conn`.
    pub fn rule_packet(&mut self, conn: u32, pass: bool) {
        let Some(held) = self.heard.take_if(|held| held.conn == conn) else {
            return;
        };
        let Some(i) = self.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        if pass {
            self.guests.update(i, |g| {
                g.inbox
                    .push_front(InboxItem::Packet(held.packet, held.credit));
            });
            self.heard_pass = Some(conn);
        }
    }
}
