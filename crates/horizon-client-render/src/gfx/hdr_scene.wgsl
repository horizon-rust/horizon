// HDR output with Linear Light (gfx/hdr.rs `present_scene`): the scene's light, in multiples of
// paper white (gfx/tonemap.rs), tone mapped to the display's peak where the HUD left the frame as
// the SDR tone mapping wrote it; the frame itself, at paper white, where the HUD, the menus or text
// drew. HDR10 (BT.2020, PQ) or scRGB (1.0 = 80 nits).
@group(0) @binding(0) var frame: texture_2d<f32>;
// The frame right after the SDR tone mapping, before the HUD (`keep_world`).
@group(0) @binding(1) var world: texture_2d<f32>;
@group(0) @binding(2) var scene: texture_2d<f32>;

struct Params {
    // Paper white in scRGB units (nits / 80).
    paper: f32,
    // Peak brightness relative to paper white; 1 expands nothing.
    peak: f32,
    // 1 when this frame drew a world.
    world: f32,
    // 1 paints light above paper white magenta (HORIZON_HDR_DEBUG=1).
    debug: f32,
    // 1 for an HDR10 swapchain, 0 for scRGB.
    pq: f32,
};
@group(0) @binding(3) var<uniform> params: Params;
// Ray-traced shadows' mask (gfx/tonemap.wgsl `shadowed`): 1x1 without them.
@group(0) @binding(4) var mask: texture_2d<f32>;
fn shadowed(c: vec4<f32>, p: vec2<f32>) -> vec3<f32> {
    let m = textureDimensions(mask);
    if m.x <= 1u || c.a > 0.75 { return c.rgb; }
    let at = vec2<i32>(p * vec2<f32>(m) / vec2<f32>(textureDimensions(scene)));
    let light = textureLoad(mask, at, 0).rg;
    let share = 2.0 * c.a;
    return c.rgb * (share * light.g + (1.0 - share) * light.r);
}

// Linear BT.709 to BT.2020 (ITU-R BT.2087), columns first.
const BT709_TO_BT2020 = mat3x3<f32>(
    vec3(0.6274, 0.0691, 0.0164),
    vec3(0.3293, 0.9195, 0.0880),
    vec3(0.0433, 0.0114, 0.8956),
);

// SMPTE ST 2084: nits (up to 10000) to the PQ signal.
fn pq(nits: vec3<f32>) -> vec3<f32> {
    let y = pow(clamp(nits / 10000.0, vec3(0.0), vec3(1.0)), vec3(0.1593017578125));
    return pow((0.8359375 + 18.8515625 * y) / (1.0 + 18.6875 * y), vec3(78.84375));
}

@vertex fn vs_hdr(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4(p * 2.0 - 1.0, 0.0, 1.0);
}

// The SDR curve's shape (tonemap.wgsl) with the display's peak in place of paper white: the
// identity up to 80 % of the peak (and never below paper white), a shoulder above it to the peak,
// half keeping the hue, half toward white.
fn shoulder(x: f32, knee: f32, top: f32) -> f32 {
    if x <= knee { return x; }
    return knee + (top - knee) * (1.0 - exp(-(x - knee) / (top - knee)));
}
fn tone_hdr(rgb: vec3<f32>, top: f32) -> vec3<f32> {
    let c = max(rgb, vec3(0.0));
    let knee = max(min(0.8 * top, top - 0.1), 0.9);
    let m = max(c.r, max(c.g, c.b));
    if m <= knee || top <= knee { return min(c, vec3(top)); }
    let hue = c * (shoulder(m, knee, top) / m);
    let white = vec3(shoulder(c.r, knee, top), shoulder(c.g, knee, top), shoulder(c.b, knee, top));
    return mix(hue, white, 0.5);
}

@fragment fn fs_hdr(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let at = vec2<i32>(pos.xy);
    let c = textureLoad(frame, at, 0).rgb;
    // Gamma 2.2, as SDR displays and the compositors' SDR paths decode it.
    var light = pow(max(c, vec3(0.0)), vec3(2.2));
    if params.world > 0.5 && all(c == textureLoad(world, at, 0).rgb) {
        let s = shadowed(textureLoad(scene, at, 0), pos.xy);
        light = tone_hdr(s, max(params.peak, 1.0));
        if params.debug > 0.5 && max(s.r, max(s.g, s.b)) > 1.0 {
            light = mix(light, vec3(params.peak, 0.0, params.peak), 0.6);
        }
    }
    let scrgb = light * params.paper;
    if params.pq > 0.5 {
        return vec4(pq(BT709_TO_BT2020 * scrgb * 80.0), 1.0);
    }
    return vec4(scrgb, 1.0);
}
