//! Play packets a mod sends one connected player, checked whole before the first goes out.
use super::Game;
use horizon_protocol::{
    buf::{Codec, Ident, Reader},
    ids::play::clientbound as ids,
    packets::{custom_payload, play::Clientbound as C},
};
use mod_api::{ClientboundPlayPacket, PlayerRef, SendPacketsRefusal as Refusal};

/// `BundlerInfo.BUNDLE_SIZE_LIMIT`: the client throws on a bundle of more packets.
const BUNDLE_LIMIT: usize = 4096;

impl Game<'_> {
    /// `packets` to that player, in order, between two bundle delimiters when `bundle` asks for
    /// one and there are two or more.
    pub(super) fn player_send_packets(
        &mut self,
        player: PlayerRef,
        packets: &[ClientboundPlayPacket],
        bundle: bool,
    ) -> Result<(), Refusal> {
        self.player_connection(player)
            .map_err(|refusal| match refusal {
                mod_api::PlayerWorldRefusal::Client => Refusal::Client,
                mod_api::PlayerWorldRefusal::NoConnection => Refusal::NoConnection,
                _ => Refusal::Stale,
            })?;
        let bundle = bundle && packets.len() > 1;
        if bundle && packets.len() > BUNDLE_LIMIT {
            return Err(Refusal::BundleTooLong {
                len: packets.len() as u32,
                limit: BUNDLE_LIMIT as u32,
            });
        }
        let owned = |channel: &str| {
            self.online_player_record(player)
                .is_some_and(|g| g.sender.payload_transport().local.owns(channel))
        };
        let mut checked = Vec::with_capacity(packets.len() + 2 * usize::from(bundle));
        for (index, packet) in packets.iter().enumerate() {
            checked.push(check(index as u32, packet, owned)?);
        }
        if bundle {
            let delimiter = || C::BundleDelimiter(horizon_protocol::packets::Empty);
            checked.insert(0, delimiter());
            checked.push(delimiter());
        }
        let uuid = player.uuid();
        for packet in checked {
            if !self.native_send(uuid, packet) {
                return Err(Refusal::Stale);
            }
        }
        Ok(())
    }
}

/// The packet as the game sends it, once its type is known, its body decodes whole and it is
/// not one the server alone sends or a protected payload.
fn check(
    index: u32,
    packet: &ClientboundPlayPacket,
    owned: impl Fn(&str) -> bool,
) -> Result<C, Refusal> {
    let id = match packet.id.namespace() {
        "minecraft" => ids::NAMES.iter().position(|name| *name == packet.id.path()),
        _ => None,
    }
    .ok_or_else(|| Refusal::UnknownPacket {
        index,
        id: packet.id.clone(),
    })? as i32;
    if matches!(id, ids::BUNDLE_DELIMITER | ids::START_CONFIGURATION) {
        return Err(Refusal::ServerOnly { index });
    }
    if id == ids::CUSTOM_PAYLOAD {
        let channel = Ident::read(&mut Reader::new(&packet.body))
            .map_err(|_| Refusal::InvalidBody { index })?;
        if custom_payload::is_reserved_channel(&channel) || owned(&channel) {
            return Err(Refusal::ProtectedPayload {
                index,
                id: mod_api::PayloadId(channel),
            });
        }
    }
    match C::decode(id, &packet.body).map_err(|_| Refusal::InvalidBody { index })? {
        // An unmodeled packet keeps its opaque body; it has not been validated.
        C::Unknown(..) => Err(Refusal::UnknownPacket {
            index,
            id: packet.id.clone(),
        }),
        checked => Ok(checked),
    }
}
