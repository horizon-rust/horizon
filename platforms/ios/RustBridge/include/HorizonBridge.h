#ifndef HORIZON_IOS_BRIDGE_H
#define HORIZON_IOS_BRIDGE_H

#include <stdbool.h>
#include <stdint.h>

uint32_t horizon_ios_host_api_version(void);
bool horizon_ios_game_is_ready(void);
int32_t horizon_ios_launch_game(void);

// Only call at process startup, BEFORE UIApplicationMain.
// This is a no-op (-1) when the lightweight Rust bridge is selected.
int32_t horizon_ios_start_original_game(const char *resources, const char *home);

// Non-owning reference to Nova's MTKView; null clears it.
void horizon_ios_set_metal_view(void *view);
bool horizon_ios_has_metal_view(void);

// Install a user-provided ZIP into a private, validated prepared-resource root.
// Error strings are UTF-8, null-terminated (when capacity > 0).
int32_t nova_ios_install_resources_zip(const char *zip_path,
                                       const char *destination_path,
                                       char *error_buffer,
                                       uintptr_t error_capacity);
bool nova_ios_resources_present(const char *destination_path);


// In-game UIKit overlay input; every control acts through Horizon's bindings.
void nova_ios_input_key(uint32_t code, bool down);
void nova_ios_input_mouse(uint32_t button, bool down);
void nova_ios_input_cursor(double x, double y);
void nova_ios_input_motion(double dx, double dy);
void nova_ios_input_text(const char *utf8);
void nova_ios_input_close(void);
void nova_ios_set_touch_gestures(bool enabled);
void nova_ios_set_overlay_active(bool active);
bool nova_ios_is_playing(void);

#endif
