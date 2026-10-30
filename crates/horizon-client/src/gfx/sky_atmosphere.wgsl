// RT lighting's sky: a WGSL port of the RTX Remix for Minecraft sky shader
// `sky_atmosphere.comp` (assets/rtx-remix-renderer/shaders/sky_atmosphere.comp, GLSL 4.30
// compute).
//
// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0
//
// Licensed under the Apache License, Version 2.0 (the "License"); you may not use this file
// except in compliance with the License. You may obtain a copy of the License at
// http://www.apache.org/licenses/LICENSE-2.0. Unless required by applicable law or agreed to in
// writing, software distributed under the License is distributed on an "AS IS" BASIS, WITHOUT
// WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied. See the License for the specific
// language governing permissions and limitations under the License.
//
// Ported to WGSL for Horizon (atmosphere.rs): the constants, the tables (a 1024x256 transmittance
// table, a 64x32 single-scattering view of 16 view steps with 16 light steps each), the analytic
// multiple scattering and the faked ground are Remix's, in the same passes. One shader, four entry
// points, one dispatch each: `build_lut` (once), `single_scattering`, `reduce_and_compose` (one
// workgroup), then `convert`, which is Horizon's own: it resamples Remix's world-fixed sky (azimuth
// around the sphere, zenith angle linear) into the sky-view table the terrain and the rays read
// (azimuth from the sun, elevation with a square-root mapping).
//
// Remix's units: radiance is the light of `SunRadiance` (and `MoonRadiance`) scattered, scaled by
// `AtmosphereScale`. Horizon's: the sun is 1 above the atmosphere, the moon `MOONLIGHT`, and the
// scale is `PAPER_WHITE * pi` (terrain.wgsl leaves Lambert's 1/pi in its units), set in
// atmosphere.rs.

struct Sky {
    // Toward the sun, xyz; its angular radius, w.
    sun_direction_and_radius: vec4<f32>,
    moon_direction_and_radius: vec4<f32>,
    // The sun's radiance, xyz; the atmosphere's scale, w.
    sun_radiance_and_atmosphere_scale: vec4<f32>,
    moon_radiance_value: vec4<f32>,
    // The Mie anisotropy, x.
    atmosphere_parameters: vec4<f32>,
}

@group(0) @binding(0) var<uniform> sky: Sky;
@group(0) @binding(1) var transmittance_lut_image: texture_storage_2d<rgba32float, write>;
@group(0) @binding(2) var transmittance_lut: texture_2d<f32>;
@group(0) @binding(3) var upper_single_image: texture_storage_2d<rgba32float, write>;
@group(0) @binding(4) var upper_view_image: texture_storage_2d<rgba32float, write>;
@group(0) @binding(5) var upper_single: texture_2d<f32>;
@group(0) @binding(6) var upper_view: texture_2d<f32>;
@group(0) @binding(7) var final_sky_image: texture_storage_2d<rgba16float, write>;
@group(0) @binding(8) var final_sky: texture_2d<f32>;
@group(0) @binding(9) var sky_view_image: texture_storage_2d<rgba16float, write>;

var<workgroup> reduction: array<vec3<f32>, 64>;
var<workgroup> reduced_upper_mean: vec3<f32>;

const SKY_WIDTH: i32 = 64;
const SKY_HEIGHT: i32 = 32;
const UPPER_HEIGHT: i32 = 16;
const VIEW_STEPS: i32 = 16;
const LIGHT_STEPS: i32 = 16;
const LUT_WIDTH: i32 = 1024;
const LUT_HEIGHT: i32 = 256;

const PI: f32 = 3.14159265358979323846;
const EARTH_RADIUS: f32 = 6371000.0;
const ATMOSPHERE_RADIUS: f32 = 6471000.0;
const ATMOSPHERE_HEIGHT: f32 = 100000.0;
const RAYLEIGH_HEIGHT: f32 = 8000.0;
const MIE_HEIGHT: f32 = 1200.0;
const BETA_R: vec3<f32> = vec3<f32>(5.5e-6, 13.0e-6, 22.4e-6);
const BETA_M: f32 = 21.0e-6;
const SUN_MIE_HALO_BOOST: f32 = 10.35;
const MOON_MIE_HALO_BOOST: f32 = 1.0;
const MULTIPLE_SCATTERING_SCALE: f32 = 0.65;
const GROUND_HORIZON_SCALE: f32 = 0.9;
const GROUND_NADIR_SCALE: f32 = 0.3;

fn ray_sphere(origin: vec3<f32>, direction: vec3<f32>, radius: f32) -> f32 {
    let b = 2.0 * dot(origin, direction);
    let c = dot(origin, origin) - radius * radius;
    let discriminant = b * b - 4.0 * c;
    if discriminant < 0.0 {
        return -1.0;
    }
    let root = sqrt(discriminant);
    let near_distance = (-b - root) * 0.5;
    let far_distance = (-b + root) * 0.5;
    if far_distance < 0.0 {
        return -1.0;
    }
    return select(near_distance, far_distance, near_distance < 0.0);
}

fn sky_direction(pixel: vec2<i32>) -> vec3<f32> {
    let u = (f32(pixel.x) + 0.5) / f32(SKY_WIDTH);
    let v = (f32(pixel.y) + 0.5) / f32(SKY_HEIGHT);
    let theta = v * PI;
    let phi = u * 2.0 * PI - PI * 0.5;
    let sin_theta = sin(theta);
    return vec3<f32>(sin_theta * cos(phi), cos(theta), sin_theta * sin(phi));
}

fn build_transmittance_lut(pixel: vec2<i32>) {
    let normalized_height = f32(pixel.y) / f32(LUT_HEIGHT - 1);
    let radius = EARTH_RADIUS + normalized_height * normalized_height * ATMOSPHERE_HEIGHT;
    let radius_ratio = EARTH_RADIUS / radius;
    let minimum_visible_cos = -sqrt(max(0.0, 1.0 - radius_ratio * radius_ratio));
    var light_cos = 2.0 * f32(pixel.x) / f32(LUT_WIDTH - 1) - 1.0;
    light_cos = max(light_cos, minimum_visible_cos);
    let light_sin = sqrt(max(0.0, 1.0 - light_cos * light_cos));
    let origin = vec3<f32>(0.0, radius, 0.0);
    let direction = vec3<f32>(light_sin, light_cos, 0.0);
    let light_distance = ray_sphere(origin, direction, ATMOSPHERE_RADIUS);
    let step_size = light_distance / f32(LIGHT_STEPS);
    var optical_r = vec3<f32>(0.0);
    var optical_m = 0.0;
    for (var i = 0; i < LIGHT_STEPS; i++) {
        let along = (f32(i) + 0.5) * step_size;
        let sample_position = origin + direction * along;
        let height = max(0.0, length(sample_position) - EARTH_RADIUS);
        let density_r = exp(-height / RAYLEIGH_HEIGHT) * step_size;
        let density_m = exp(-height / MIE_HEIGHT) * step_size;
        optical_r += BETA_R * density_r;
        optical_m += BETA_M * density_m;
    }
    textureStore(
        transmittance_lut_image,
        pixel,
        vec4<f32>(exp(-(optical_r + vec3<f32>(optical_m))), 1.0),
    );
}

// The table read bilinearly at texel coordinates `at` (clamped to the edge), as a filtering
// sampler would: rgba32float is not filterable without a feature, so by hand.
fn lut_bilinear(at: vec2<f32>) -> vec3<f32> {
    let p = at - 0.5;
    let base = floor(p);
    let f = p - base;
    let lo = vec2<i32>(base);
    let top = vec2<i32>(LUT_WIDTH - 1, LUT_HEIGHT - 1);
    let a = textureLoad(transmittance_lut, clamp(lo, vec2<i32>(0), top), 0).rgb;
    let b = textureLoad(transmittance_lut, clamp(lo + vec2<i32>(1, 0), vec2<i32>(0), top), 0).rgb;
    let c = textureLoad(transmittance_lut, clamp(lo + vec2<i32>(0, 1), vec2<i32>(0), top), 0).rgb;
    let d = textureLoad(transmittance_lut, clamp(lo + vec2<i32>(1, 1), vec2<i32>(0), top), 0).rgb;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// The light's transmittance at `position`, w 0 when the Earth is in the way.
fn sample_light(position: vec3<f32>, radius: f32, light_direction: vec3<f32>) -> vec4<f32> {
    let light_cos = dot(position, light_direction) / radius;
    if light_cos < 0.0 && radius * radius * (1.0 - light_cos * light_cos) < EARTH_RADIUS * EARTH_RADIUS {
        return vec4<f32>(0.0);
    }
    let height = max(0.0, radius - EARTH_RADIUS);
    let height_coordinate = sqrt(min(1.0, height / ATMOSPHERE_HEIGHT)) * f32(LUT_HEIGHT - 1);
    let angle_coordinate = (clamp(light_cos, -1.0, 1.0) + 1.0) * 0.5 * f32(LUT_WIDTH - 1);
    // Remix's uv * size - 0.5 + 0.5: the texel coordinates plus a half, as bilinear reads them.
    return vec4<f32>(lut_bilinear(vec2<f32>(angle_coordinate, height_coordinate) + 0.5), 1.0);
}

fn accumulate_light(
    position: vec3<f32>,
    radius: f32,
    density_r: f32,
    density_m: f32,
    view_transmittance: vec3<f32>,
    light_direction: vec3<f32>,
    rayleigh: ptr<function, vec3<f32>>,
    mie: ptr<function, vec3<f32>>,
) {
    let light = sample_light(position, radius, light_direction);
    if light.w == 0.0 {
        return;
    }
    let attenuation = view_transmittance * light.rgb;
    *rayleigh += density_r * BETA_R * attenuation;
    *mie += density_m * BETA_M * attenuation;
}

fn finish_scattering(
    view_direction: vec3<f32>,
    light_direction: vec3<f32>,
    light_radiance: vec3<f32>,
    rayleigh: vec3<f32>,
    mie: vec3<f32>,
    mie_halo_boost: f32,
) -> vec3<f32> {
    let cos_angle = dot(view_direction, light_direction);
    let phase_r = 3.0 / (16.0 * PI) * (1.0 + cos_angle * cos_angle);
    let anisotropy = sky.atmosphere_parameters.x;
    let g2 = anisotropy * anisotropy;
    let phase_m = 3.0 / (8.0 * PI)
        * ((1.0 - g2) * (1.0 + cos_angle * cos_angle))
        / ((2.0 + g2) * pow(1.0 + g2 - 2.0 * anisotropy * cos_angle, 1.5));
    return (rayleigh * phase_r + mie * phase_m * mie_halo_boost)
        * light_radiance * sky.sun_radiance_and_atmosphere_scale.w;
}

fn generate_single_scattering(pixel: vec2<i32>) {
    let sun_direction = sky.sun_direction_and_radius.xyz;
    let moon_direction = sky.moon_direction_and_radius.xyz;
    let sun_radiance = sky.sun_radiance_and_atmosphere_scale.xyz;
    let moon_radiance = sky.moon_radiance_value.xyz;
    let view_direction = sky_direction(pixel);
    let origin = vec3<f32>(0.0, EARTH_RADIUS, 0.0);
    let atmosphere_distance = ray_sphere(origin, view_direction, ATMOSPHERE_RADIUS);
    let step_size = atmosphere_distance / f32(VIEW_STEPS);
    var optical_r = vec3<f32>(0.0);
    var optical_m = 0.0;
    var sun_rayleigh = vec3<f32>(0.0);
    var sun_mie = vec3<f32>(0.0);
    var moon_rayleigh = vec3<f32>(0.0);
    var moon_mie = vec3<f32>(0.0);

    for (var i = 0; i < VIEW_STEPS; i++) {
        let along = (f32(i) + 0.5) * step_size;
        let sample_position = origin + view_direction * along;
        let radius = length(sample_position);
        let height = max(0.0, radius - EARTH_RADIUS);
        let density_r = exp(-height / RAYLEIGH_HEIGHT) * step_size;
        let density_m = exp(-height / MIE_HEIGHT) * step_size;
        optical_r += BETA_R * density_r;
        optical_m += BETA_M * density_m;
        let view_transmittance = exp(-(optical_r + vec3<f32>(optical_m)));
        accumulate_light(
            sample_position, radius, density_r, density_m, view_transmittance,
            sun_direction, &sun_rayleigh, &sun_mie,
        );
        accumulate_light(
            sample_position, radius, density_r, density_m, view_transmittance,
            moon_direction, &moon_rayleigh, &moon_mie,
        );
    }

    let view_transmittance = exp(-(optical_r + vec3<f32>(optical_m)));
    var single_scattering = finish_scattering(
        view_direction, sun_direction, sun_radiance, sun_rayleigh, sun_mie, SUN_MIE_HALO_BOOST,
    );
    single_scattering += finish_scattering(
        view_direction, moon_direction, moon_radiance, moon_rayleigh, moon_mie, MOON_MIE_HALO_BOOST,
    );

    let sun_cos = dot(view_direction, sun_direction);
    if acos(clamp(sun_cos, -1.0, 1.0)) < sky.sun_direction_and_radius.w {
        single_scattering += sun_radiance * view_transmittance;
    }
    let moon_cos = dot(view_direction, moon_direction);
    if acos(clamp(moon_cos, -1.0, 1.0)) < sky.moon_direction_and_radius.w {
        single_scattering += moon_radiance * view_transmittance;
    }

    textureStore(upper_single_image, pixel, vec4<f32>(single_scattering, 1.0));
    textureStore(upper_view_image, pixel, vec4<f32>(view_transmittance, 1.0));
}

@compute @workgroup_size(8, 8)
fn build_lut(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    if pixel.x < LUT_WIDTH && pixel.y < LUT_HEIGHT {
        build_transmittance_lut(pixel);
    }
}

@compute @workgroup_size(8, 8)
fn single_scattering(@builtin(global_invocation_id) id: vec3<u32>) {
    let pixel = vec2<i32>(id.xy);
    if pixel.x < SKY_WIDTH && pixel.y < UPPER_HEIGHT {
        generate_single_scattering(pixel);
    }
}

// One workgroup of 64: the upper hemisphere's mean from 32 samples, the multiple scattering
// added, the average sky (the sky-ambient light) at (0, SKY_HEIGHT), the faked ground below.
@compute @workgroup_size(8, 8)
fn reduce_and_compose(@builtin(local_invocation_index) invocation: u32) {
    var sum = vec3<f32>(0.0);
    if invocation < 32u {
        let sample_index = i32(invocation);
        sum = textureLoad(
            upper_single,
            vec2<i32>(
                (sample_index % 8) * SKY_WIDTH / 8,
                (sample_index / 8) * UPPER_HEIGHT / 4,
            ),
            0,
        ).rgb;
    }
    reduction[invocation] = sum;
    workgroupBarrier();
    for (var stride = 32u; stride > 0u; stride >>= 1u) {
        if invocation < stride {
            reduction[invocation] += reduction[invocation + stride];
        }
        workgroupBarrier();
    }
    if invocation == 0u {
        reduced_upper_mean = reduction[0] / 32.0;
    }
    workgroupBarrier();
    let upper_mean = reduced_upper_mean;

    var final_sum = vec3<f32>(0.0);
    for (var index = invocation; index < u32(SKY_WIDTH * UPPER_HEIGHT); index += 64u) {
        let pixel = vec2<i32>(i32(index) % SKY_WIDTH, i32(index) / SKY_WIDTH);
        let single_scattering = textureLoad(upper_single, pixel, 0).rgb;
        let view_transmittance = textureLoad(upper_view, pixel, 0).rgb;
        let radiance = max(
            single_scattering
                + upper_mean * (vec3<f32>(1.0) - view_transmittance) * MULTIPLE_SCATTERING_SCALE,
            vec3<f32>(0.0),
        );
        let quantized_rg = unpack2x16float(pack2x16float(radiance.rg));
        let quantized_ba = unpack2x16float(pack2x16float(vec2<f32>(radiance.b, 1.0)));
        final_sum += vec3<f32>(quantized_rg, quantized_ba.x);
        textureStore(final_sky_image, pixel, vec4<f32>(radiance, 1.0));
    }
    workgroupBarrier();
    reduction[invocation] = final_sum;
    workgroupBarrier();
    for (var stride = 32u; stride > 0u; stride >>= 1u) {
        if invocation < stride {
            reduction[invocation] += reduction[invocation + stride];
        }
        workgroupBarrier();
    }
    if invocation == 0u {
        textureStore(
            final_sky_image,
            vec2<i32>(0, SKY_HEIGHT),
            vec4<f32>(reduction[0] / f32(SKY_WIDTH * UPPER_HEIGHT), 1.0),
        );
    }
    workgroupBarrier();

    for (var x = i32(invocation); x < SKY_WIDTH; x += 64) {
        let horizon_pixel = vec2<i32>(x, UPPER_HEIGHT - 1);
        let horizon = textureLoad(upper_single, horizon_pixel, 0).rgb
            + upper_mean
                * (vec3<f32>(1.0) - textureLoad(upper_view, horizon_pixel, 0).rgb)
                * MULTIPLE_SCATTERING_SCALE;
        for (var y = UPPER_HEIGHT; y < SKY_HEIGHT; y++) {
            let amount = f32(y - UPPER_HEIGHT) / f32(SKY_HEIGHT - UPPER_HEIGHT - 1);
            let scale = mix(GROUND_HORIZON_SCALE, GROUND_NADIR_SCALE, amount);
            textureStore(
                final_sky_image,
                vec2<i32>(x, y),
                vec4<f32>(max(horizon * scale, vec3<f32>(0.0)), 1.0),
            );
        }
    }
}

fn final_texel(at: vec2<i32>) -> vec3<f32> {
    // The azimuth wraps, the zenith angle is clamped (the average sky's row is not the sky's).
    let x = ((at.x % SKY_WIDTH) + SKY_WIDTH) % SKY_WIDTH;
    return textureLoad(final_sky, vec2<i32>(x, clamp(at.y, 0, SKY_HEIGHT - 1)), 0).rgb;
}

// Remix's sky toward `d` (unit, y up, the world's frame), bilinear.
fn final_sky_toward(d: vec3<f32>) -> vec3<f32> {
    let phi = atan2(d.z, d.x);
    let u = fract((phi + PI * 0.5) / (2.0 * PI));
    let v = acos(clamp(d.y, -1.0, 1.0)) / PI;
    let p = vec2<f32>(u * f32(SKY_WIDTH), v * f32(SKY_HEIGHT)) - 0.5;
    let base = floor(p);
    let f = p - base;
    let lo = vec2<i32>(base);
    let low = mix(final_texel(lo), final_texel(lo + vec2<i32>(1, 0)), f.x);
    let high = mix(final_texel(lo + vec2<i32>(0, 1)), final_texel(lo + vec2<i32>(1, 1)), f.x);
    return mix(low, high, f.y);
}

// Horizon's sky-view table (atmosphere.rs `SIZE`): azimuth from the sun 0..pi, elevation
// -pi/2..pi/2 with a square-root mapping, Remix's sky resampled.
@compute @workgroup_size(8, 8)
fn convert(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = vec2<i32>(textureDimensions(sky_view_image));
    let pixel = vec2<i32>(id.xy);
    if pixel.x >= size.x || pixel.y >= size.y {
        return;
    }
    let sun = sky.sun_direction_and_radius.xyz;
    var toward = vec2<f32>(1.0, 0.0);
    if length(sun.xz) > 1e-6 {
        toward = normalize(sun.xz);
    }
    let v = (f32(pixel.y) + 0.5) / f32(size.y) * 2.0 - 1.0;
    let elevation = sign(v) * v * v * (PI * 0.5);
    let azimuth = (f32(pixel.x) + 0.5) / f32(size.x) * PI;
    // The frame: toward the sun's side, and Y cross that.
    let flat_direction = vec2<f32>(toward.x, toward.y) * cos(azimuth)
        + vec2<f32>(toward.y, -toward.x) * sin(azimuth);
    let d = vec3<f32>(flat_direction.x * cos(elevation), sin(elevation), flat_direction.y * cos(elevation));
    textureStore(sky_view_image, pixel, vec4<f32>(final_sky_toward(d), 1.0));
}
