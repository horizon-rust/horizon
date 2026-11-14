// What the material and entity stages share. The look (hdr_look.wgsl) works on colours in
// vanilla's gamma, as Horizon's own shaders hand it them; a stage's colours are linear light.

// A stage's linear light as the colour vanilla draws.
fn gamma(light: vec3<f32>) -> vec3<f32> {
    return pow(max(light, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.2));
}

// The look's tier, 1 low to 4 very bright and 0 for what gives no light, of a kind of light as
// `MaterialInput.emission` and `EntityInput.light_source` give it: 7/15, 14/15, 1 and past 1.
fn tier_of(kind: f32) -> u32 {
    return u32(kind > 0.0) + u32(kind > 0.5) + u32(kind > 0.95) + u32(kind > 1.5);
}

// The light of a pixel drawn `color` under `mark` (`hdr_mark`; 1 lifts nothing). In SDR nothing
// is lifted, and the pixel is vanilla's.
fn lit(color: vec3<f32>, mark: f32) -> vec3<f32> {
    return hdr_lit(color, mark, frame.hdr == 1u).light;
}
