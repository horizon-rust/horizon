//! Owned data for client play-packet submission; protocol decoding stays in the game.

use crate::Identifier;

/// A serverbound play packet encoded in the connected game's vanilla protocol.
///
/// The game resolves the type name and validates its body once at submission. The packet
/// reserves outgoing credit and becomes visible only when the current root transaction
/// commits. A phase change or disconnect before publication cancels it. Registered and
/// engine-reserved custom payload channels require their payload/service API instead.
/// This is not a connection or protocol implementation; submission applies no local effects.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ServerboundPlayPacket {
    /// Packet type, for example `minecraft:pong` or `minecraft:custom_payload`.
    pub id: Identifier,
    /// Vanilla packet body, without the packet id, length prefix or compression framing.
    pub body: Vec<u8>,
}

impl ServerboundPlayPacket {
    /// Construct a packet envelope; the game validates the body when it is submitted.
    pub fn new(id: Identifier, body: Vec<u8>) -> Self {
        Self { id, body }
    }
}

/// A clientbound play packet encoded in the vanilla protocol, for the server to send a player.
///
/// The game resolves the type name and checks the body decodes completely before anything
/// is sent. It is the clientbound side of [`ServerboundPlayPacket`]: the same envelope, owned
/// by the mod API, so no protocol type is part of it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct ClientboundPlayPacket {
    /// Packet type, for example `minecraft:set_entity_data` or `minecraft:set_time`.
    pub id: Identifier,
    /// Vanilla packet body, without the packet id, length prefix or compression framing.
    pub body: Vec<u8>,
}

impl ClientboundPlayPacket {
    /// Construct a packet envelope; the game validates the body when it is sent.
    pub fn new(id: Identifier, body: Vec<u8>) -> Self {
        Self { id, body }
    }
}

/// Why `player.send_packets` sent nothing: a refusal leaves the whole list unsent.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SendPacketsRefusal {
    /// The call was made on a client; these are the server's.
    Client,
    /// The player handle no longer identifies a player in the server's list.
    Stale,
    /// The player has no connection to send to (a fake player).
    NoConnection,
    /// The packet at `index` has no modeled clientbound play packet of that type name:
    /// unknown names and types without a body decoder are both refused.
    UnknownPacket {
        /// Its position in the list.
        index: u32,
        /// The type name given.
        id: Identifier,
    },
    /// The body of the packet at `index` does not decode completely as the named packet.
    InvalidBody {
        /// Its position in the list.
        index: u32,
    },
    /// The packet at `index` is one the server alone sends: `minecraft:bundle_delimiter` (ask
    /// for a bundle instead) or `minecraft:start_configuration` (it ends the play state, and a
    /// bundle holding it makes the client throw).
    ServerOnly {
        /// Its position in the list.
        index: u32,
    },
    /// The packet at `index` is a custom payload on a channel a registered payload or an engine
    /// service owns: use the payload or service API.
    ProtectedPayload {
        /// Its position in the list.
        index: u32,
        /// The protected channel.
        id: crate::PayloadId,
    },
    /// A bundle of more than `limit` packets (`BundlerInfo.BUNDLE_SIZE_LIMIT`) is refused by the
    /// client.
    BundleTooLong {
        /// The number of packets given.
        len: u32,
        /// The most a bundle holds.
        limit: u32,
    },
}

impl crate::NoBackend for SendPacketsRefusal {
    fn no_backend() -> Self {
        Self::Stale
    }
}

/// Why the client could not submit a play packet.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum SendPacketError {
    /// There is no server connection.
    NotConnected,
    /// The player has not joined, or the connection is reconfiguring.
    NotInPlay,
    /// The channel belongs to a registered payload or an engine service. Use its
    /// payload/service API rather than bypassing negotiation through raw packets.
    ProtectedPayload {
        /// The protected custom-payload channel.
        id: crate::PayloadId,
    },
    /// No outgoing resource credit remains; nothing was queued.
    Backpressure,
    /// The encoded or uncompressed packet exceeds this physical connection's framing bound.
    FrameTooLarge {
        /// Packet bytes at the refused framing stage.
        size: u64,
        /// Maximum bytes accepted at that stage.
        limit: u64,
    },
    /// The physical encoder could not prepare the packet.
    EncodeFailed {
        /// The encoder's diagnostic.
        message: String,
    },
    /// This protocol has no serverbound packet with the given type name.
    UnknownPacket {
        /// The submitted type name.
        id: Identifier,
    },
    /// The body does not decode completely as the named play packet.
    InvalidBody,
    /// A custom payload exceeds vanilla's size limit.
    PayloadTooLarge {
        /// Number of payload bytes, excluding the channel name.
        size: u64,
        /// Maximum number of payload bytes vanilla accepts.
        limit: u64,
    },
}

/// Why a client mod's `disconnect` was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum DisconnectError {
    /// No server or world is joined: there is nothing to leave.
    Unavailable,
    /// The reason exceeds the text boundary's size or nesting limits.
    Text(crate::TextFormError),
}

impl From<crate::TextFormError> for DisconnectError {
    fn from(error: crate::TextFormError) -> Self {
        Self::Text(error)
    }
}

impl crate::NoBackend for DisconnectError {
    /// No game: nothing is joined.
    fn no_backend() -> Self {
        Self::Unavailable
    }
}
