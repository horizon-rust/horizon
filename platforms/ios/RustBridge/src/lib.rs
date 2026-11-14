//! iOS host ABI scaffold. Deliberately does not substitute or alter gameplay.
//! When Horizon's iOS runtime integration is proven, wire these entry points
//! into the ORIGINAL upstream Horizon client, not a copied game implementation.

#[no_mangle]
pub extern "C" fn horizon_ios_host_api_version() -> u32 {
    1
}

#[no_mangle]
pub extern "C" fn horizon_ios_game_is_ready() -> bool {
    false
}

/// Returns an explicit not-integrated error instead of claiming a game launch.
#[no_mangle]
pub extern "C" fn horizon_ios_launch_game() -> i32 {
    -1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_status_is_explicit() {
        assert_eq!(horizon_ios_host_api_version(), 1);
        assert!(!horizon_ios_game_is_ready());
        assert_eq!(horizon_ios_launch_game(), -1);
    }
}
