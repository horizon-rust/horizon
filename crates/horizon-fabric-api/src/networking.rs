//! Custom payload channel registry, mirroring Fabric's networking API.

use std::collections::HashMap;

use crate::registry::{Identifier, RegistryError};

/// Which side a channel carries payloads on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelDirection {
    /// Client to server.
    ClientToServer,
    /// Server to client.
    ServerToClient,
    /// Both directions.
    Both,
}

/// A custom payload sent over a channel.
#[derive(Debug, Clone)]
pub struct CustomPayload {
    /// The channel.
    pub channel: Identifier,
    /// Raw payload bytes.
    pub data: Vec<u8>,
}

/// Registry of custom payload channels declared by mods.
#[derive(Debug, Default)]
pub struct ChannelRegistry {
    channels: HashMap<Identifier, ChannelDirection>,
}

impl ChannelRegistry {
    /// Create an empty channel registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a channel. Re-declaring an existing channel widens its direction.
    pub fn register(
        &mut self,
        channel: Identifier,
        direction: ChannelDirection,
    ) -> Result<(), RegistryError> {
        use ChannelDirection::*;
        match self.channels.get(&channel).copied() {
            Some(existing) if existing == direction || existing == Both => Ok(()),
            Some(existing) if existing != direction => {
                let _ = existing;
                self.channels.insert(channel, Both);
                Ok(())
            }
            _ => {
                self.channels.insert(channel, direction);
                Ok(())
            }
        }
    }

    /// Whether a channel is declared.
    pub fn contains(&self, channel: &Identifier) -> bool {
        self.channels.contains_key(channel)
    }

    /// The direction a channel was declared with.
    pub fn direction(&self, channel: &Identifier) -> Option<ChannelDirection> {
        self.channels.get(channel).copied()
    }

    /// Every declared channel.
    pub fn iter(&self) -> impl Iterator<Item = (&Identifier, ChannelDirection)> {
        self.channels.iter().map(|(id, direction)| (id, *direction))
    }

    /// Number of declared channels.
    pub fn len(&self) -> usize {
        self.channels.len()
    }

    /// Whether no channels are declared.
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }

    /// Validate that a payload may be sent in the given direction.
    pub fn accepts(&self, channel: &Identifier, direction: ChannelDirection) -> bool {
        match self.channels.get(channel).copied() {
            Some(ChannelDirection::Both) => true,
            Some(declared) => declared == direction,
            None => false,
        }
    }
}
