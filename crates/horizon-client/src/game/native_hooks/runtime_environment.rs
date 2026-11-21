//! Metadata reads use the bootstrap snapshot, never the world's logical side.
use super::Game;

impl Game<'_> {
    pub(in crate::game) fn dist(&self) -> mod_api::Dist {
        crate::entry::runtime_environment::get().dist()
    }
    pub(in crate::game) fn game_version(&self) -> &'static str {
        crate::entry::runtime_environment::get().game_version()
    }
    pub(in crate::game) fn server_data_version(&self) -> i32 {
        horizon_core::version::DATA_VERSION
    }
    pub(in crate::game) fn game_protocol_version(&self) -> i32 {
        horizon_core::version::PROTOCOL_VERSION
    }
    pub(in crate::game) fn protocol_name(&self, protocol: i32) -> Option<&'static str> {
        protocol_name_of(protocol)
    }
    pub(in crate::game) fn horizon_version(&self) -> &'static str {
        crate::entry::runtime_environment::get().horizon_version()
    }
    pub(in crate::game) fn game_tests_enabled(&self) -> bool {
        crate::entry::runtime_environment::get().game_tests_enabled()
    }
}

/// The version name of `protocol`: only this build's own protocol has one (`WorldVersion.name()`).
pub(crate) fn protocol_name_of(protocol: i32) -> Option<&'static str> {
    (protocol == horizon_core::version::PROTOCOL_VERSION).then_some(horizon_core::version::VANILLA_VERSION)
}

#[cfg(test)]
#[path = "runtime_environment/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "runtime_environment/server_data_version_tests.rs"]
mod server_data_version_tests;

#[cfg(test)]
#[path = "runtime_environment/protocol_version_tests.rs"]
mod protocol_version_tests;
