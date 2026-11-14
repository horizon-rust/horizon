//! One of Horizon's own shader packs while it runs (docs/shaders.md): the frame its stages
//! read, its sky, material and post stages, and the timer of their passes.

use horizon_shader_api::{Report, StageKind};

use super::displace::Displacement;
use super::native_material::{DisplacedShape, NativeMaterial, TerrainShape};
use super::native_post::{NativeFrame, NativePost};
use super::native_shadow::{NativeShadow, ShadowGrid};
use super::native_sky::NativeSky;
use super::pack_timing::PassTiming;

/// Runs `create` and returns its validation error, if any, or the backend's (a shader the
/// driver's compiler refuses is an internal error). A panic while it runs is an error too:
/// naga's backends turn a module they cannot write into a panic as the driver gets the pipeline
/// (#3525), which a pack's program must not turn into a crash.
pub(super) fn checked<T>(device: &wgpu::Device, create: impl FnOnce() -> T) -> Result<T, String> {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let value = crate::crash::contain(create);
    let internal = pollster::block_on(internal.pop());
    match pollster::block_on(validation.pop()).or(internal) {
        Some(e) => {
            // The details are in the error's description or its sources.
            let mut message = match &e {
                wgpu::Error::Validation { source, .. } => format!("{source:?}"),
                e => e.to_string(),
            };
            let mut source = std::error::Error::source(&e);
            while let Some(s) = source {
                message = format!("{message}: {s}");
                source = s.source();
            }
            Err(message)
        }
        None => value.map_err(|panic| format!("the shader compiler failed: {panic}")),
    }
}

pub struct NativePack {
    pub frame: NativeFrame,
    pub sky: Option<NativeSky>,
    pub material: Option<NativeMaterial>,
    pub post: Option<NativePost>,
    /// With a sky stage, what of Minecraft's sky still draws over it (`[sky]`).
    pub vanilla_clouds: bool,
    pub vanilla_sun: bool,
    pub vanilla_stars: bool,
    pub weather: Option<wgpu::RenderPipeline>,
    /// The draw of the shadow map the stages read, if the pack asks for one.
    pub shadow: Option<NativeShadow>,
    /// `[displacement]`'s grid, while it is on and the resource packs have height maps.
    pub displacement: Option<Displacement>,
    /// `[[particle_lights]]`.
    pub particle_lights: Vec<horizon_shader_api::manifest::ParticleLight>,
    /// `[[lights]]`, for the colour of held and dropped items' light.
    pub light_colors: Vec<horizon_shader_api::manifest::LightColor>,
    /// GPU time of each pass, while the renderer asks for it (`pack_timing.rs`).
    pub timing: Option<PassTiming>,
}

/// Whether the option `id` (a bool, or a number above 0) is on, or there is none to ask.
fn option_on(constants: &[(String, f64)], id: Option<&String>) -> bool {
    id.is_none_or(|id| {
        constants
            .iter()
            .find(|(k, _)| k == id)
            .is_none_or(|(_, v)| *v != 0.0)
    })
}

/// `[displacement]`'s cells: the value of its `cells_option`, if any.
fn chosen_cells(
    d: &horizon_shader_api::manifest::Displacement,
    options: &[horizon_shader_api::manifest::PackOption],
    constants: &[(String, f64)],
) -> u32 {
    let chosen = d.cells_option.as_ref().and_then(|id| {
        let option = options.iter().find(|o| o.id == *id)?;
        let horizon_shader_api::manifest::OptionKind::Choice { values, .. } = &option.kind else {
            return None;
        };
        let (_, index) = constants.iter().find(|(k, _)| k == id)?;
        values.get(*index as usize)?.parse().ok()
    });
    chosen.unwrap_or(d.cells)
}

/// `[shadow]` with the option values `constants`: none while its `option` is off, else the
/// resolution and distance its `resolution_option` and `distance_option` pick.
fn chosen_shadow(
    s: &horizon_shader_api::manifest::Shadow,
    options: &[horizon_shader_api::manifest::PackOption],
    constants: &[(String, f64)],
) -> Option<horizon_shader_api::manifest::Shadow> {
    let number = |id: Option<&String>| {
        let option = options.iter().find(|o| Some(&o.id) == id)?;
        let (_, value) = constants.iter().find(|(k, _)| *k == option.id)?;
        option.kind.number(*value)
    };
    option_on(constants, s.option.as_ref()).then(|| horizon_shader_api::manifest::Shadow {
        resolution: number(s.resolution_option.as_ref()).map_or(s.resolution, |r| r as u32),
        distance: number(s.distance_option.as_ref()).map_or(s.distance, |d| d as f32),
        ..s.clone()
    })
}

impl NativePack {
    /// The pack of the checked stages `report` with the option values `constants`, drawing
    /// onto the world's targets as `terrain` describes; the sky reads the terrain's group 0.
    /// Also returns its stages' labels, in the order they draw. Errors name the stage wgpu
    /// refused.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain: &TerrainShape<'_>,
        report: Report,
        constants: &[(String, f64)],
        lights: super::block_light::LightTable,
    ) -> Result<(Self, Vec<String>), String> {
        // A backend may refuse a layout or pipeline the pack asks for (a stage's bindings
        // past its sampled-texture limit, say, as OpenGL's 16 was before #10095): the pack
        // is refused with that error instead of a validation panic in the renderer.
        checked(device, || {
            Self::build(device, queue, terrain, report, constants, lights)
        })?
    }

    fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        terrain: &TerrainShape<'_>,
        report: Report,
        constants: &[(String, f64)],
        lights: super::block_light::LightTable,
    ) -> Result<(Self, Vec<String>), String> {
        let (globals, format) = (terrain.globals, terrain.format);
        let textures = super::native_post::pack_textures(device, queue, &report);
        // `[shadow]`, unless its option is off, at the size its options pick.
        let chosen = report
            .shadow
            .as_ref()
            .and_then(|s| chosen_shadow(s, &report.manifest.options, constants));
        let settings = chosen
            .as_ref()
            .map(|settings| super::native_shadow::fitted(device, settings));
        let api = report.manifest.api;
        let mut frame = NativeFrame::new(
            device,
            settings.as_ref(),
            textures,
            lights,
            terrain.pbr,
            (api, terrain.linear && api == horizon_shader_api::Api::V0_2),
        )?;
        // `[displacement]`, unless its option is off; nothing to displace without heights.
        let displacement = report
            .manifest
            .displacement
            .as_ref()
            .filter(|d| option_on(constants, d.option.as_ref()))
            .filter(|_| terrain.pbr.is_some())
            .map(|d| {
                // No more cells than the height maps have texels.
                let texels = terrain.pbr.map_or(16, |p| p[0].width());
                let cells = chosen_cells(d, &report.manifest.options, constants)
                    .min(texels)
                    .max(1);
                let steps =
                    d.steps_option.is_some() && option_on(constants, d.steps_option.as_ref());
                Displacement::new(
                    device,
                    d.distance,
                    cells,
                    steps,
                    &frame.pbr[0],
                    report.shadow.is_some(),
                )
            });
        let shadow = chosen
            .as_ref()
            .map(|s| {
                let vertex = report.stages.iter().find(|s| s.kind == StageKind::Vertex);
                // `[displacement]`'s grid casts too, from the material's module.
                let material = report.stages.iter().find(|s| s.kind == StageKind::Material);
                let grid = displacement.as_ref().and_then(|d| {
                    let picks = d.shadow.as_ref()?;
                    Some(ShadowGrid {
                        distance: d.distance,
                        cells: picks.cells,
                        steps: picks.steps,
                        material: material?,
                        layout: &d.draw_layout,
                        normals: &frame.pbr[0],
                    })
                });
                NativeShadow::new(
                    device,
                    terrain,
                    &frame,
                    &s.solid_materials,
                    vertex,
                    constants,
                    grid,
                )
            })
            .transpose()?;
        // The mirror is drawn for a material that reads it, while `[reflection] option` is on.
        let option = report.reflection.as_ref().and_then(|r| r.option.as_ref());
        frame.reflects = report
            .stages
            .iter()
            .find(|s| s.kind == StageKind::Material)
            .is_some_and(|s| s.reads("reflection"))
            && option_on(constants, option);
        let fog = report.material_fog;
        let keeps = |key: fn(&horizon_shader_api::Stage) -> Option<bool>| {
            report.sky.as_ref().and_then(key).unwrap_or(true)
        };
        let vanilla_clouds = keeps(|s| s.clouds);
        let vanilla_sun = keeps(|s| s.sun);
        let vanilla_stars = keeps(|s| s.stars);
        let weather = report
            .stages
            .iter()
            .find(|s| s.kind == StageKind::Weather)
            .map(|stage| {
                super::weather::native_pipeline(
                    device,
                    format,
                    terrain.weather_groups[0],
                    terrain.weather_groups[1],
                    &frame,
                    stage,
                    constants,
                )
            })
            .transpose()?;
        let weather_label = report
            .stages
            .iter()
            .find(|s| s.kind == StageKind::Weather)
            .map(|s| s.label.clone());
        let has_vertex = report.stages.iter().any(|s| s.kind == StageKind::Vertex);
        let (post, others): (Vec<_>, Vec<_>) = report
            .stages
            .into_iter()
            .partition(|s| s.kind == StageKind::Post);
        let sky = others
            .iter()
            .find(|s| s.kind == StageKind::Sky)
            .map(|stage| NativeSky::new(device, globals, &frame, stage, constants, format))
            .transpose()?;
        let material = others
            .iter()
            .find(|s| s.kind == StageKind::Material)
            .map(|stage| {
                let displaced = displacement.as_ref().map(|d| DisplacedShape {
                    distance: d.distance,
                    cells: d.view.cells,
                    steps: d.view.steps,
                    layout: &d.draw_layout,
                });
                NativeMaterial::new(
                    device,
                    terrain,
                    &frame,
                    stage,
                    constants,
                    fog,
                    has_vertex,
                    displaced.as_ref(),
                )
            })
            .transpose()?;
        // A 0.2 pack always has one: its decode and tonemap run without a chain too.
        let tonemap = others.iter().find(|s| s.kind == StageKind::Tonemap);
        let post = (!post.is_empty() || api == horizon_shader_api::Api::V0_2)
            .then(|| {
                let manifest = &report.manifest;
                NativePost::new(device, post, manifest, constants, format, &frame, tonemap)
            })
            .transpose()?;
        let mut labels: Vec<String> = sky.iter().map(|s| s.label.clone()).collect();
        labels.extend(
            others
                .iter()
                .filter(|s| material.is_some() && s.kind == StageKind::Material)
                .map(|s| s.label.clone()),
        );
        labels.extend(weather_label);
        if let Some(post) = &post {
            labels.extend(post.labels().into_iter().map(str::to_owned));
        }
        let pack = Self {
            particle_lights: report.manifest.particle_lights.clone(),
            light_colors: report.manifest.lights.clone(),
            frame,
            sky,
            material,
            post,
            vanilla_clouds,
            vanilla_sun,
            vanilla_stars,
            weather,
            shadow,
            displacement,
            timing: None,
        };
        Ok((pack, labels))
    }

    /// Whether the pack draws anything the game runs today.
    pub fn draws(&self) -> bool {
        self.sky.is_some()
            || self.material.is_some()
            || self.weather.is_some()
            || self.post.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::{NativePack, TerrainShape, chosen_shadow};
    use horizon_shader_api::manifest::{OptionKind, PackOption, Shadow};

    /// A device with OpenGL's minimum 16 sampled textures per stage refuses a pack whose
    /// sky adds the terrain's 10 to the frame's 9: that is the pack's error to report, not
    /// a validation panic (#10095).
    #[test]
    fn a_pack_the_backend_refuses_is_an_error_not_a_panic() {
        let Some((device, queue)) = limited_device() else {
            return;
        };
        let built = test_pack(&device, &queue, 10, 0, None);
        let error = built.err().expect("the pack should be refused");
        assert!(
            error.contains("max_sampled_textures_per_shader_stage"),
            "{error}"
        );
    }

    #[test]
    fn custom_textures_respect_the_device_limit_without_poisoning_the_next_pack() {
        let Some((device, queue)) = limited_device() else {
            return;
        };
        // The frame has nine textures. Seven globals fit exactly; adding a custom
        // texture exceeds the combined pipeline limit. Eight custom textures also
        // exceed the frame group's limit even without globals. A valid pack still loads.
        for (globals, textures, requested) in [(7, 0, 16), (7, 1, 17), (0, 8, 17), (7, 0, 16)] {
            let built = test_pack(&device, &queue, globals, textures, None);
            if requested == 16 {
                assert!(built.is_ok(), "{}", built.err().unwrap());
            } else {
                let error = built.err().expect("an oversized pack must return Err");
                assert!(
                    error.contains("max_sampled_textures_per_shader_stage"),
                    "{error}"
                );
                assert!(error.contains("17") && error.contains("16"), "{error}");
            }
        }
    }

    #[test]
    fn post_targets_count_toward_the_sampled_texture_limit() {
        let Some((device, queue)) = limited_device() else {
            return;
        };
        // Eight built-in post textures, seven custom textures and one target fit.
        // A second target must be refused, even when the shader does not sample it.
        for targets in [1, 2, 1] {
            let built = test_pack(&device, &queue, 0, 7, Some(targets));
            if targets == 1 {
                assert!(built.is_ok(), "{}", built.err().unwrap());
            } else {
                let error = built.err().expect("too many post textures must return Err");
                assert!(
                    error.contains("max_sampled_textures_per_shader_stage"),
                    "{error}"
                );
                assert!(error.contains("17") && error.contains("16"), "{error}");
            }
        }
    }

    #[test]
    fn a_compiler_panic_is_an_error_and_leaves_the_scopes_usable() {
        let Some((device, _queue)) = limited_device() else {
            return;
        };
        let outer = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result: Result<(), String> = super::checked(&device, || panic!("test shader compiler"));
        let error = result.unwrap_err();
        assert!(error.starts_with("the shader compiler failed:"), "{error}");
        assert!(error.contains("test shader compiler"), "{error}");
        assert_eq!(super::checked(&device, || 42).unwrap(), 42);
        assert!(pollster::block_on(outer.pop()).is_none());
    }

    fn limited_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = crate::gfx::test_gpu::instance();
        let Ok(adapter) = pollster::block_on(instance.request_adapter(&Default::default())) else {
            eprintln!("skipping native pack GPU test: no adapter");
            return None;
        };
        let mut limits = adapter.limits();
        limits.max_sampled_textures_per_shader_stage = 16;
        let Ok((device, queue)) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                required_limits: limits,
                ..Default::default()
            }))
        else {
            eprintln!("skipping native pack GPU test: no device");
            return None;
        };
        Some((device, queue))
    }

    fn test_pack(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        globals_count: u32,
        textures: u32,
        post_targets: Option<u32>,
    ) -> Result<(NativePack, Vec<String>), String> {
        use horizon_shader_api::{MANIFEST, Pack, check};
        // Synthetic globals let the test reach both sides of the pipeline's limit
        // independently of the adapter's limits and the renderer's sprite array count.
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty,
            count: None,
        };
        let mut entries = vec![entry(
            0,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        )];
        entries.extend((1..=globals_count).map(|binding| {
            entry(
                binding,
                wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
            )
        }));
        let globals = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &entries,
        });
        let empty = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[],
        });
        let terrain = TerrainShape {
            globals: &globals,
            flat: None,
            sprites: [1.0, 2048.0],
            format: wgpu::TextureFormat::Rgba8Unorm,
            linear: false,
            sprite_tags: &[],
            sprite_materials: &[],
            entities: &empty,
            weather_groups: [&empty, &empty],
            pbr: None,
        };
        let mut manifest =
            String::from("schema = 1\nid = \"t\"\nname = \"T\"\nversion = \"1\"\napi = \"0.1\"\n");
        let mut files = std::collections::BTreeMap::new();
        if let Some(targets) = post_targets {
            for i in 0..=targets {
                let name = format!("post_{i}.wgsl");
                manifest.push_str(&format!("[[post]]\nfile = \"{name}\"\n"));
                if i < targets {
                    manifest.push_str(&format!("output = \"target_{i}\"\n"));
                }
                files.insert(
                    name,
                    b"fn post(input: PostInput) -> vec4<f32> { return input.color; }".to_vec(),
                );
            }
        } else {
            manifest.push_str("[sky]\nfile = \"sky.wgsl\"\n");
            files.insert(
                "sky.wgsl".to_owned(),
                b"fn sky(input: SkyInput) -> vec4<f32> { return input.color; }".to_vec(),
            );
        }
        for i in 0..textures {
            manifest.push_str(&format!(
                "[[textures]]\nid = \"custom_{i}\"\nfile = \"texture_{i}.png\"\n"
            ));
        }
        files.insert(MANIFEST.to_owned(), manifest.into_bytes());
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[255; 4])
                .unwrap();
        }
        for i in 0..textures {
            files.insert(format!("texture_{i}.png"), png.clone());
        }
        let report = check(&Pack::from_files(files).unwrap()).unwrap();
        NativePack::new(device, queue, &terrain, report, &[], Default::default())
    }

    fn option(id: &str, kind: OptionKind) -> PackOption {
        PackOption {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            kind,
        }
    }

    #[test]
    fn the_shadow_map_follows_its_options() {
        let options = [
            option("shadows", OptionKind::Bool { default: true }),
            option(
                "quality",
                OptionKind::Choice {
                    values: vec!["1024".into(), "4096".into()],
                    default: 0,
                },
            ),
            option(
                "reach",
                OptionKind::Int {
                    default: 96,
                    min: 32,
                    max: 512,
                },
            ),
        ];
        let shadow = Shadow {
            option: Some("shadows".into()),
            resolution_option: Some("quality".into()),
            distance_option: Some("reach".into()),
            ..Shadow::default()
        };
        let values = |on: f64, quality: f64, reach: f64| {
            [("shadows", on), ("quality", quality), ("reach", reach)].map(|(k, v)| (k.into(), v))
        };
        let size = |values: &[(String, f64)]| {
            chosen_shadow(&shadow, &options, values).map(|s| (s.resolution, s.distance))
        };
        assert_eq!(size(&values(1.0, 1.0, 200.0)), Some((4096, 200.0)));
        assert_eq!(size(&values(1.0, 0.0, 64.0)), Some((1024, 64.0)));
        assert_eq!(size(&values(0.0, 1.0, 200.0)), None);
        // Without the options' keys: the manifest's own numbers, always drawn.
        assert_eq!(
            chosen_shadow(&Shadow::default(), &options, &values(0.0, 1.0, 200.0))
                .map(|s| (s.resolution, s.distance)),
            Some((2048, 128.0))
        );
    }
}
