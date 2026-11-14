//! RT lighting's sun and sky (docs/raytracing.md): the real sun's light through the atmosphere and
//! the sky's light scattered by it, in place of vanilla's sun and sky colours. The atmosphere is
//! RTX Remix for Minecraft's, ported (`sky_atmosphere.wgsl` from its `sky_atmosphere.comp`,
//! Apache-2.0): Rayleigh 8 km and Mie 1.2 km scattering on an Earth of 6371 km under 100 km of
//! air, a 1024x256 transmittance table, a 64x32 single-scattering view of 16 view and 16 light
//! steps, and the ground below the horizon faked. The sunlight reaching the ground is its
//! transmittance along the sun's elevation, white overhead and red at sunset.
//!
//! Three things are not Remix's any more, each checked against a spectral reference (the sun's
//! spectrum through Rayleigh's lambda^-4 and the ozone's measured cross-sections, 36 wavelengths,
//! seen through the CIE observer in sRGB) and each with its switch back ([`Air`]): the air's
//! coefficients, the view march, and the multiple scattering (Hillaire's 2020 table).
//!
//! The compute passes run on the GPU when the sun has moved (a few times a minute), the
//! transmittance table once. Remix's sky is world-fixed (azimuth around the sphere, the zenith
//! angle linear); a last pass resamples it into the table the terrain's light, the rays and the
//! fog read (`SIZE`, azimuth from the sun by elevation with a square-root mapping).
//!
//! Units: Remix scales its radiance by `SunRadiance` and `AtmosphereScale`; here the sun's light
//! above the atmosphere is 1 (the moon's `MOONLIGHT`), and the scale `PAPER_WHITE * pi`
//! (terrain.wgsl's diffuse light leaves Lambert's 1/pi in its units): a white surface facing the
//! sun overhead comes out at about 1.1 paper white, and the sky drawn and the light it gives
//! agree.

use glam::{Vec3, Vec4};
use wgpu::util::DeviceExt;

/// The sky-view table: azimuth from the sun (0..pi), elevation (-pi/2..pi/2, finer at the horizon).
pub const SIZE: [u32; 2] = [64, 32];
/// The light scaled to the scene's: paper white for the sun's light above the atmosphere.
pub const PAPER_WHITE: f32 = 1.25;
/// The moon's light against the sun's (gameplay's, not the real 1/400000): moonlit nights stay
/// readable.
pub const MOONLIGHT: f32 = 0.025;
/// [`MOONLIGHT`], or `HORIZON_MOONLIGHT`'s (0 to 1; 0.05 was the value before: a full moon lit the
/// ground as a dull day).
pub fn moonlight() -> f32 {
    static LIGHT: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *LIGHT.get_or_init(|| {
        std::env::var("HORIZON_MOONLIGHT")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| (0.0..=1.0).contains(v))
            .unwrap_or(MOONLIGHT)
    })
}
/// What a new moon leaves of the night sky's light, as a fraction of a full moon's: the airglow, the
/// starlight and the zodiacal light, enough to tell the ground from the sky and too little to hide
/// a star.
const NIGHT_GLOW: f32 = 0.1;

// Remix's constants (sky_atmosphere.comp), in metres.
const EARTH_RADIUS: f32 = 6_371_000.0;
const ATMOSPHERE_RADIUS: f32 = 6_471_000.0;
const RAYLEIGH_HEIGHT: f32 = 8000.0;
const MIE_HEIGHT: f32 = 1200.0;
/// The air's coefficients and how the sky's passes add its light up, with the switches back to
/// what they were (read once).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Air {
    /// Rayleigh's scattering a metre at sea level, red, green and blue.
    pub rayleigh: Vec3,
    /// The ozone's absorption a metre where it is densest.
    pub ozone: Vec3,
    /// The aerosols' scattering a metre at sea level: a clear day's (Hillaire's 4e-6, where
    /// Remix's 21e-6 is a hazy one).
    pub mie: f32,
    /// What the sun's Mie halo is multiplied by.
    pub halo: f32,
    /// Remix's view march.
    pub old_march: bool,
    /// Remix's multiple scattering's scale, 0 for the table's.
    pub old_multiple: f32,
}

/// The ground's albedo for the multiple scattering's table (Hillaire's 0.3).
const GROUND_ALBEDO: f32 = 0.3;

impl Air {
    /// The air's coefficients as the sRGB primaries see the spectrum: the thin-layer limit of
    /// sunlight (a 5778 K black body, white-balanced) scattered by Rayleigh's 1.24062e-6 /
    /// lambda^4 (micrometres) and absorbed by the ozone's Chappuis band (300 Dobson units over
    /// 15 km). Against the spectral sky these keep its colour within 2 % from noon to the end of
    /// the civil twilight. The usual three-wavelength values (680, 550 and 440 nm: 5.8, 13.5 and
    /// 33.1e-6, ozone 0.65, 1.88 and 0.085e-6) miss what the red primary sees of the orange
    /// (where the ozone absorbs most): the low sun came out 40 % too bright in red and the
    /// twilight's zenith purple instead of blue.
    const SPECTRAL: (Vec3, Vec3) = (
        Vec3::new(6.621e-6, 13.721e-6, 32.321e-6),
        Vec3::new(2.517e-6, 1.794e-6, 0.0),
    );
    /// What they were (`HORIZON_SKY_AIR=old`): a blue a third under Rayleigh's (22.4 against
    /// 32.3: a paler sky, a sunset less red), the ozone at three wavelengths.
    const OLD: (Vec3, Vec3) = (
        Vec3::new(5.5e-6, 13.0e-6, 22.4e-6),
        Vec3::new(0.650e-6, 1.881e-6, 0.085e-6),
    );

    /// The air: `HORIZON_SKY_AIR=old` for the coefficients before, `HORIZON_SKY_MARCH=old` for
    /// Remix's view march (steps of one length and the sun's halo doubled to make up for the
    /// aerosols it missed) and `HORIZON_SKY_MULTIPLE=old` for Remix's multiple scattering (the
    /// upper sky's mean by 0.65: about 40 % less light from the sky by day, half the blue).
    pub fn get() -> Self {
        static AIR: std::sync::OnceLock<Air> = std::sync::OnceLock::new();
        *AIR.get_or_init(|| {
            let old = |name: &str| std::env::var(name).is_ok_and(|v| v.trim() == "old");
            Self::with(
                old("HORIZON_SKY_AIR"),
                old("HORIZON_SKY_MARCH"),
                old("HORIZON_SKY_MULTIPLE"),
            )
        })
    }

    fn with(old_air: bool, old_march: bool, old_multiple: bool) -> Self {
        let (rayleigh, ozone) = if old_air { Self::OLD } else { Self::SPECTRAL };
        Self {
            rayleigh,
            ozone,
            mie: 6.0e-6,
            halo: if old_march { 2.0 } else { 1.0 },
            old_march,
            old_multiple: if old_multiple { 0.65 } else { 0.0 },
        }
    }
}
const LIGHT_STEPS: usize = 16;
/// The Mie scattering's anisotropy (`AtmosphereParameters.x`, which Remix's mod sets; not in the
/// shader): the usual 0.8.
const MIE_ANISOTROPY: f32 = 0.8;

/// Remix's tables: the transmittance table, and the sky's (one more row holds its average).
const LUT_SIZE: [u32; 2] = [1024, 256];
const MULTIPLE_SIZE: [u32; 2] = [32, 32];
const UPPER_SIZE: [u32; 2] = [64, 16];
const FINAL_SIZE: [u32; 2] = [64, 33];

/// How far a ray from `origin` (radius `r`, along unit `direction` at the zenith cosine `mu`) goes
/// before it leaves the atmosphere.
fn atmosphere_distance(r: f32, mu: f32) -> f32 {
    let (b, c) = (2.0 * r * mu, r * r - ATMOSPHERE_RADIUS * ATMOSPHERE_RADIUS);
    let discriminant = b * b - 4.0 * c;
    (-b + discriminant.max(0.0).sqrt()) * 0.5
}

/// The sky shader's transmittance table, one texel, on the CPU: from outside to radius `r` along
/// the zenith cosine `mu`, 16 steps (`build_transmittance_lut`).
fn transmittance_lut(air: &Air, r: f32, mu: f32) -> Vec3 {
    let sin = (1.0 - mu * mu).max(0.0).sqrt();
    let length = atmosphere_distance(r, mu);
    let step = length / LIGHT_STEPS as f32;
    let (mut optical_r, mut optical_m) = (Vec3::ZERO, 0.0);
    for i in 0..LIGHT_STEPS {
        let along = (i as f32 + 0.5) * step;
        // The point `along` the ray from (0, r, 0) toward (sin, mu, 0).
        let point = Vec3::new(sin * along, r + mu * along, 0.0);
        let height = (point.length() - EARTH_RADIUS).max(0.0);
        let ozone = (1.0 - (height - 25_000.0).abs() / 15_000.0).max(0.0);
        optical_r +=
            air.rayleigh * ((-height / RAYLEIGH_HEIGHT).exp() * step) + air.ozone * (ozone * step);
        optical_m += air.mie * (-height / MIE_HEIGHT).exp() * step;
    }
    (-(optical_r + Vec3::splat(optical_m))).exp()
}

/// The `Sky` uniform of `sky_atmosphere.wgsl` (Remix's `SkyParameters`).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Sky {
    sun_direction_and_radius: [f32; 4],
    moon_direction_and_radius: [f32; 4],
    sun_radiance_and_atmosphere_scale: [f32; 4],
    moon_radiance_value: [f32; 4],
    atmosphere_parameters: [f32; 4],
    rayleigh: [f32; 4],
    ozone: [f32; 4],
    mie: [f32; 4],
}

/// RT lighting's atmosphere: Remix's sky tables and the passes that make them.
pub struct Atmosphere {
    device: wgpu::Device,
    #[cfg(test)]
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    uniform: wgpu::Buffer,
    lut: Pass,
    multiple: Pass,
    single: Pass,
    reduce: Pass,
    convert: Pass,
    /// The transmittance table is made.
    lut_made: bool,
    /// The sun's direction, the moon's lit fraction and the rain's cover the sky-view table holds.
    made_for: Option<(Vec3, f32, Vec4)>,
}

/// A compute pass: its pipeline, its bindings and the workgroups it runs.
struct Pass {
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
    groups: [u32; 2],
    label: &'static str,
}

impl Atmosphere {
    pub fn new(device: &wgpu::Device) -> Self {
        let table = |label, size: [u32; 2], format, extra: wgpu::TextureUsages| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | extra,
                view_formats: &[],
            })
        };
        use wgpu::TextureFormat::{Rgba16Float, Rgba32Float};
        let lut = table(
            "Sky transmittance",
            LUT_SIZE,
            Rgba32Float,
            wgpu::TextureUsages::empty(),
        );
        let multiple = table(
            "Sky multiple scattering",
            MULTIPLE_SIZE,
            Rgba32Float,
            wgpu::TextureUsages::empty(),
        );
        let upper_single = table(
            "Sky single scattering",
            UPPER_SIZE,
            Rgba32Float,
            wgpu::TextureUsages::empty(),
        );
        let upper_view = table(
            "Sky view transmittance",
            UPPER_SIZE,
            Rgba32Float,
            wgpu::TextureUsages::empty(),
        );
        let final_sky = table(
            "Sky, Remix's",
            FINAL_SIZE,
            Rgba16Float,
            wgpu::TextureUsages::empty(),
        );
        let texture = table(
            "RT lighting's sky",
            SIZE,
            Rgba16Float,
            wgpu::TextureUsages::COPY_SRC,
        );
        let view_of = |t: &wgpu::Texture| t.create_view(&Default::default());
        let multiple_v = view_of(&multiple);
        let (lut_v, single_v, view_v, final_v, view) = (
            view_of(&lut),
            view_of(&upper_single),
            view_of(&upper_view),
            view_of(&final_sky),
            view_of(&texture),
        );
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sky parameters"),
            contents: bytemuck::bytes_of(&Sky::for_sun(Vec3::Y, 1.0)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Sky atmosphere"),
            source: wgpu::ShaderSource::Wgsl(include_str!("sky_atmosphere.wgsl").into()),
        });

        enum Bind<'a> {
            Uniform,
            Texture(&'a wgpu::TextureView),
            Store(&'a wgpu::TextureView, wgpu::TextureFormat),
        }
        let pass =
            |label: &'static str, entry: &str, bindings: &[(u32, Bind)], groups: [u32; 2]| {
                let layout_entries: Vec<_> = bindings
                    .iter()
                    .map(|(binding, kind)| wgpu::BindGroupLayoutEntry {
                        binding: *binding,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: match kind {
                            Bind::Uniform => wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            Bind::Texture(_) => wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            Bind::Store(_, format) => wgpu::BindingType::StorageTexture {
                                access: wgpu::StorageTextureAccess::WriteOnly,
                                format: *format,
                                view_dimension: wgpu::TextureViewDimension::D2,
                            },
                        },
                        count: None,
                    })
                    .collect();
                let group_layout =
                    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some(label),
                        entries: &layout_entries,
                    });
                let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: &[Some(&group_layout)],
                    immediate_size: 0,
                });
                let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                    label: Some(label),
                    layout: Some(&layout),
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    cache: None,
                });
                let entries: Vec<_> = bindings
                    .iter()
                    .map(|(binding, kind)| wgpu::BindGroupEntry {
                        binding: *binding,
                        resource: match kind {
                            Bind::Uniform => uniform.as_entire_binding(),
                            Bind::Texture(v) | Bind::Store(v, _) => {
                                wgpu::BindingResource::TextureView(v)
                            }
                        },
                    })
                    .collect();
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout: &group_layout,
                    entries: &entries,
                });
                Pass {
                    pipeline,
                    bind_group,
                    groups,
                    label,
                }
            };
        let lut_pass = pass(
            "Sky transmittance table",
            "build_lut",
            &[(0, Bind::Uniform), (1, Bind::Store(&lut_v, Rgba32Float))],
            [LUT_SIZE[0] / 8, LUT_SIZE[1] / 8],
        );
        let multiple_pass = pass(
            "Sky multiple scattering table",
            "build_multiple",
            &[
                (0, Bind::Uniform),
                (2, Bind::Texture(&lut_v)),
                (10, Bind::Store(&multiple_v, Rgba32Float)),
            ],
            [MULTIPLE_SIZE[0] / 8, MULTIPLE_SIZE[1] / 8],
        );
        let single = pass(
            "Sky single scattering",
            "single_scattering",
            &[
                (0, Bind::Uniform),
                (2, Bind::Texture(&lut_v)),
                (3, Bind::Store(&single_v, Rgba32Float)),
                (4, Bind::Store(&view_v, Rgba32Float)),
                (11, Bind::Texture(&multiple_v)),
            ],
            [UPPER_SIZE[0] / 8, UPPER_SIZE[1] / 8],
        );
        let reduce = pass(
            "Sky reduce and compose",
            "reduce_and_compose",
            &[
                (0, Bind::Uniform),
                (5, Bind::Texture(&single_v)),
                (6, Bind::Texture(&view_v)),
                (7, Bind::Store(&final_v, Rgba16Float)),
            ],
            [1, 1],
        );
        let convert = pass(
            "Sky view table",
            "convert",
            &[
                (0, Bind::Uniform),
                (8, Bind::Texture(&final_v)),
                (9, Bind::Store(&view, Rgba16Float)),
            ],
            [SIZE[0] / 8, SIZE[1] / 8],
        );
        Self {
            device: device.clone(),
            #[cfg(test)]
            texture,
            view,
            uniform,
            lut: lut_pass,
            multiple: multiple_pass,
            single,
            reduce,
            convert,
            lut_made: false,
            made_for: None,
        }
    }

    /// The sunlight reaching the ground from direction `sun` (unit, y up), above the atmosphere
    /// 1: the transmittance table's value at sea level, 0 once the Earth hides the sun.
    pub fn sunlight(&self, sun: Vec3) -> Vec3 {
        if sun.y <= 0.0 {
            return Vec3::ZERO;
        }
        transmittance_lut(&Air::get(), EARTH_RADIUS, sun.y)
    }

    /// Makes the sky-view table again when the sun (toward `sun`, unit, y up) has moved since by
    /// more than a tenth of a degree, or the moon's lit fraction `moon_lit` (0 new, 1 full) has
    /// changed: Remix's passes on the GPU, in a command buffer of their own, ahead of the frame's.
    /// `overcast` is the rain's cloud cover ([`overcast`]): the zenith's light (xyz) and how much of
    /// the sky it covers (w); when only it changed, only the last pass runs again.
    pub fn update(&mut self, queue: &wgpu::Queue, sun: Vec3, moon_lit: f32, overcast: Vec4) {
        let same_sun = self.made_for.is_some_and(|(last, lit, _)| {
            last.dot(sun) > 0.999_998 && (lit - moon_lit).abs() < 1e-3
        });
        let same_cover = self.made_for.is_some_and(|(_, _, last)| {
            (last.w - overcast.w).abs() < 2e-3
                && (last.truncate() - overcast.truncate()).length()
                    <= 0.01 * overcast.truncate().length()
        });
        if same_sun && same_cover {
            return;
        }
        self.made_for = Some((sun, moon_lit, overcast));
        let mut sky = Sky::for_sun(sun, moon_lit);
        sky.moon_radiance_value[3] = overcast.w;
        sky.atmosphere_parameters[1..].copy_from_slice(&overcast.truncate().to_array());
        queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&sky));
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Sky atmosphere"),
            });
        let lut = std::mem::replace(&mut self.lut_made, true);
        let passes = [
            (!lut).then_some(&self.lut),
            (!lut).then_some(&self.multiple),
            (!same_sun).then_some(&self.single),
            (!same_sun).then_some(&self.reduce),
            Some(&self.convert),
        ];
        for pass in passes.into_iter().flatten() {
            let mut compute = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some(pass.label),
                timestamp_writes: None,
            });
            compute.set_pipeline(&pass.pipeline);
            compute.set_bind_group(0, &pass.bind_group, &[]);
            compute.dispatch_workgroups(pass.groups[0], pass.groups[1], 1);
        }
        queue.submit([encoder.finish()]);
    }
}

/// The rain's cloud cover over the sky table (`Atmosphere::update`): a CIE overcast sky (the
/// zenith three times as bright as the horizon, the same all round) over as much of the sky as the
/// rain level `rain` says (all of it from two thirds), letting through `through` of the light the
/// clear day gives a level surface (`level_light`, paper whites: the sun's or the moon's on it and
/// the clear sky's): a raining stratus deck about a third, a storm's cumulonimbus about a tenth. The zenith's light
/// (xyz, slightly blue grey) is such that a level surface under it gets that, and w the cover.
pub fn overcast(rain: f32, storm: f32, level_light: f32) -> Vec4 {
    let cover = (rain * 1.5).clamp(0.0, 1.0);
    let through = overcast_through(storm);
    // A CIE overcast sky lights a level surface with 7/9 of its zenith's radiance times pi (the
    // pi is in the table's units).
    let zenith = through * level_light / (7.0 / 9.0);
    (Vec3::new(0.93, 0.98, 1.06) * zenith).extend(cover)
}

/// The share of a clear day's light the rain's clouds let through (`overcast`).
fn overcast_through(storm: f32) -> f32 {
    0.35 + (0.12 - 0.35) * storm.clamp(0.0, 1.0)
}

/// The stops the rain's cloud cover takes off the light (`overcast`): the exposure's top under
/// Remix's rain and thunder (set for a light it does not dim) is raised by as much.
pub fn overcast_stops(rain: f32, storm: f32) -> f32 {
    -overcast_through(storm).log2() * (rain * 1.5).clamp(0.0, 1.0)
}

impl Sky {
    /// Remix's `SkyParameters` for the sun toward `sun` and the moon opposite, its light by its
    /// phase (`moon_lit`, the fraction of its face lit): the moon scatters in the air like the sun,
    /// so a full moon lifts the night sky and hides the faint stars, a new moon leaves it dark.
    /// The discs' angular radii are 0: the sky table is 5.6 degrees a texel, a disc of the sun's
    /// size would light a texel now and then; the sun and the moon are drawn on their own.
    fn for_sun(sun: Vec3, moon_lit: f32) -> Self {
        let moon_light = moonlight() * moon_lit.max(NIGHT_GLOW);
        let air = Air::get();
        Self {
            sun_direction_and_radius: [sun.x, sun.y, sun.z, 0.0],
            moon_direction_and_radius: [-sun.x, -sun.y, -sun.z, 0.0],
            sun_radiance_and_atmosphere_scale: [1.0, 1.0, 1.0, PAPER_WHITE * std::f32::consts::PI],
            moon_radiance_value: [moon_light, moon_light, moon_light, 0.0],
            atmosphere_parameters: [MIE_ANISOTROPY, 0.0, 0.0, 0.0],
            rayleigh: air.rayleigh.extend(air.halo).to_array(),
            ozone: air
                .ozone
                .extend(f32::from(u8::from(air.old_march)))
                .to_array(),
            mie: [air.mie, air.old_multiple, GROUND_ALBEDO, 0.0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sun_is_white_overhead_and_red_at_sunset() {
        let air = Air::get();
        let noon = transmittance_lut(&air, EARTH_RADIUS, 1.0);
        assert!(noon.min_element() > 0.6 && noon.x > noon.z, "{noon}");
        let low = transmittance_lut(&air, EARTH_RADIUS, 0.03);
        assert!(low.x > 3.0 * low.z, "{low}");
    }

    fn test_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?;
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
    }

    /// The sky table the GPU makes for `sun`, as f32 texels.
    fn gpu_sky(device: &wgpu::Device, queue: &wgpu::Queue, sun: Vec3) -> Vec<[f32; 4]> {
        let mut atmosphere = Atmosphere::new(device);
        atmosphere.update(queue, sun, 1.0, Vec4::ZERO);
        let row = SIZE[0] * 8;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("sky readback"),
            size: u64::from(row * SIZE[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            atmosphere.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(SIZE[1]),
                },
            },
            wgpu::Extent3d {
                width: SIZE[0],
                height: SIZE[1],
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = buffer.slice(..).get_mapped_range().unwrap();
        let halves: &[u16] = bytemuck::cast_slice(&data);
        let half = |h: u16| {
            let (sign, exponent, mantissa) = (h >> 15, (h >> 10) & 31, h & 1023);
            let v = match exponent {
                0 => f32::from(mantissa) / 1024.0 * 2f32.powi(-14),
                31 => f32::INFINITY,
                e => (1.0 + f32::from(mantissa) / 1024.0) * 2f32.powi(i32::from(e) - 15),
            };
            if sign == 1 { -v } else { v }
        };
        halves
            .chunks(4)
            .map(|t| [half(t[0]), half(t[1]), half(t[2]), half(t[3])])
            .collect()
    }

    /// The reference: the sun's spectrum (a 5778 K black body) through Rayleigh's lambda^-4, the
    /// ozone's measured cross-sections and the aerosols, 36 wavelengths, seen by the CIE observer
    /// in sRGB balanced on the sun; worked out apart from this code.
    #[test]
    fn the_sun_s_colour_is_the_spectrum_s() {
        let air = Air::with(false, false, false);
        for (elevation, spectral) in [
            (90.0f32, [0.907, 0.866, 0.767]),
            (30.0, [0.824, 0.749, 0.589]),
            (10.0, [0.592, 0.447, 0.228]),
            (5.0, [0.398, 0.233, 0.064]),
        ] {
            let light = transmittance_lut(&air, EARTH_RADIUS, elevation.to_radians().sin());
            let off = (light - Vec3::from(spectral)).abs().max_element();
            assert!(off < 0.02, "{elevation}: {light} against {spectral:?}");
        }
    }

    /// The sky's light against the same spectral reference's single scattering plus Hillaire's
    /// multiple scattering worked out apart (Python, finer steps), at texels (row, column) of the
    /// table: near the zenith toward and away from the sun, mid-sky and near the horizon.
    #[test]
    fn the_sky_s_colour_is_the_spectrum_s() {
        let Some((device, queue)) = test_device() else {
            eprintln!("no GPU adapter: the sky's passes are not run");
            return;
        };
        let texel = |sky: &[[f32; 4]], row: usize, column: usize| {
            Vec3::from_slice(&sky[row * SIZE[0] as usize + column][..3])
        };
        for (elevation, expected) in [
            (
                60.0f32,
                vec![
                    ((31, 0), [0.0400, 0.0710, 0.1465]),
                    ((31, 63), [0.0321, 0.0615, 0.1338]),
                    ((26, 63), [0.0338, 0.0678, 0.1502]),
                    ((19, 32), [0.1839, 0.2883, 0.4058]),
                ],
            ),
            (
                5.0,
                vec![
                    ((31, 63), [0.0082, 0.0154, 0.0285]),
                    ((26, 63), [0.0181, 0.0326, 0.0540]),
                    ((19, 32), [0.0688, 0.0864, 0.0750]),
                ],
            ),
        ] {
            let (sin, cos) = elevation.to_radians().sin_cos();
            let sky = gpu_sky(&device, &queue, Vec3::new(cos, sin, 0.0));
            for ((row, column), reference) in expected {
                let (got, reference) = (texel(&sky, row, column), Vec3::from(reference));
                let off = ((got - reference) / reference).abs().max_element();
                assert!(
                    off < 0.1,
                    "sun {elevation}, texel {row} {column}: {got} against {reference}"
                );
            }
        }
    }

    /// The table's texel high in the sky, away from the sun: the last azimuth, the sixth row from
    /// the top.
    fn zenith(texels: &[[f32; 4]]) -> [f32; 4] {
        texels[(SIZE[1] as usize - 6) * SIZE[0] as usize + SIZE[0] as usize - 1]
    }

    #[test]
    fn the_sky_is_blue_by_day_and_dark_at_night() {
        let Some((device, queue)) = test_device() else {
            eprintln!("no GPU adapter: the sky's passes are not run");
            return;
        };
        let sun = Vec3::new(0.3, 0.9, 0.1).normalize();
        let day = zenith(&gpu_sky(&device, &queue, sun));
        assert!(day[2] > day[0] && day[2] > 0.05 && day[2] < 1.5, "{day:?}");
        let night = zenith(&gpu_sky(&device, &queue, -sun));
        assert!(night[2] < 0.1 * day[2], "{night:?} {day:?}");
        let sky = gpu_sky(&device, &queue, sun);
        assert_eq!(sky.len(), (SIZE[0] * SIZE[1]) as usize);
        assert!(
            sky.iter()
                .all(|t| t[..3].iter().all(|c| c.is_finite() && *c >= 0.0))
        );
    }
}
