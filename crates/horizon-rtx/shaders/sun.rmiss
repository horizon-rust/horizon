#version 460
// The sun pass's miss: the ray reached the sun (sun.rgen reads the hit object, not this).
#extension GL_EXT_ray_tracing : require
layout(location = 0) rayPayloadInEXT float payload;
void main() {
    payload = 1.0;
}
