#ifndef HORIZON_IOS_BRIDGE_H
#define HORIZON_IOS_BRIDGE_H

#include <stdbool.h>
#include <stdint.h>

uint32_t horizon_ios_host_api_version(void);
bool horizon_ios_game_is_ready(void);
int32_t horizon_ios_launch_game(void);

// Non-owning reference to Nova's MTKView; null clears it.
void horizon_ios_set_metal_view(void *view);
bool horizon_ios_has_metal_view(void);

#endif
