//! First-run setup for the mobile build, with no desktop launcher:
//!
//! 1. [`msa`]: sign in with a Microsoft account (the device-code flow: the player
//!    types a short code at microsoft.com/link on any browser), then Xbox Live, XSTS
//!    and the Minecraft services, and check that the account owns Minecraft: Java Edition.
//! 2. [`download`]: the player's own download of the pinned version from Mojang (version
//!    JSON, client and server jars, libraries and assets), each file checked against the
//!    sha1 Mojang lists, in the official launcher's folder layout.
//!
//! Nothing of Mojang's is part of Horizon. Tokens never go into logs or error text.
#![forbid(unsafe_code)]

pub mod download;
pub mod instances;
pub mod java;
pub mod modrinth;
pub mod msa;
pub mod notes;
pub mod registry;
pub mod store;
