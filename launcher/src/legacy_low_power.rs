use super::*;

fn force_legacy_low_power_mode() {
    // GPUI Fast uses wgpu. Restrict Linux startup to wgpu's GL backend so an
    // OpenGL 3.3-only laptop cannot fall through to an unavailable Vulkan driver.
    #[cfg(target_os = "linux")]
    {
        std::env::set_var("WGPU_BACKEND", "gl");
        std::env::set_var("HORIZON_LAUNCHER_MOTION", "off");
    }
}
