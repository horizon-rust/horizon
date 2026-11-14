//! Original synthetic geometry visibility controls; no pack shader code or assets.
use super::*;
use glam::Mat4;
use wgpu::TextureFormat as T;

#[test]
#[ignore = "requires an offscreen GPU adapter; setup failure is a failure"]
fn pack_sun_and_moon_properties_control_independent_pixels() {
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("offscreen adapter");
    eprintln!("Sky visibility adapter: {:?}", adapter.get_info());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: adapter.limits(),
        ..Default::default()
    }))
    .expect("offscreen device");
    let dir = std::env::temp_dir().join(format!("hz-sky-visibility-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("shaders")).unwrap();
    std::fs::write(
        dir.join("shaders/gbuffers_skytextured.vsh"),
        "#version 120\nvoid main(){gl_Position=gl_Vertex;gl_FrontColor=gl_Color;}",
    )
    .unwrap();
    std::fs::write(
        dir.join("shaders/gbuffers_skytextured.fsh"),
        "#version 120\nvoid main(){gl_FragData[0]=gl_Color;}",
    )
    .unwrap();
    let pack = horizon_shaders::Pack::open(&dir).unwrap();
    let macros = horizon_shaders::Macros::standard();
    let inputs = horizon_shaders::GameInputs {
        celestial: Some(super::super::pack_sky::celestial_inputs()),
        ..Default::default()
    };
    let hud = crate::hud::HudBatch::new([8, 1]);
    let vertices: Vec<_> = [(-1.0, 0.0, 0xff0000ff), (0.0, 1.0, 0xff00ff00)]
        .into_iter()
        .flat_map(|(left, right, color)| {
            [
                [left, -1.0, 0.0],
                [right, -1.0, 0.0],
                [right, 1.0, 0.0],
                [left, 1.0, 0.0],
            ]
            .map(|pos| super::super::Vertex {
                pos,
                color,
                uv: [0.0; 2],
                layer: 0,
                light: 0,
                normal: [0; 4],
                glint_uv: [0.0; 2],
            })
        })
        .collect();
    let frame = frame(&hud, &vertices);
    let red = [255, 0, 0];
    let green = [0, 255, 0];
    let black = [0, 0, 0];
    for (properties, expected) in [
        ("", [red, green]),
        ("sun=false", [black, green]),
        ("moon=false", [red, black]),
        ("sun=false\nmoon=false", [black, black]),
        ("sun=true\nmoon=true", [red, green]),
    ] {
        let properties = horizon_shaders::Properties::parse(properties, &macros);
        let loaded = horizon_shaders::load_post(&pack, "", &properties, &macros, &inputs);
        assert!(loaded.sky_textured.is_some(), "{:?}", loaded.notes);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: T::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let world = World {
            atlas: None,
            sprite_rects: &[],
            material_maps: None,
            lightmap: texture.create_view(&Default::default()),
            terrain: None,
            block_ids: &[],
            celestial: None,
            gui_size: [8, 1],
            clouds: None,
            weather: false,
            hand: false,
        };
        let mut post = PackPost::new(
            &device,
            &queue,
            &adapter,
            loaded,
            T::Rgba8Unorm,
            world,
            &Default::default(),
        )
        .unwrap_or_else(|e| panic!("pack refused: {:?}", e.notes));
        post.copied |= 1;
        post.resize(&device, [8, 1]);
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let targets = post.targets.as_ref().unwrap();
            let view = &targets.color[0].as_ref().unwrap()[0];
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[attach(
                    view,
                    wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                )],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &targets.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
        }
        post.draw_sky(&device, &queue, &mut encoder, [8, 1], &frame);
        let read = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            post.targets.as_ref().unwrap().color[0].as_ref().unwrap()[0]
                .texture()
                .as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &read,
                layout: wgpu::TexelCopyBufferLayout {
                    bytes_per_row: Some(256),
                    ..Default::default()
                },
            },
            wgpu::Extent3d {
                width: 8,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        read.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let data = read.slice(..).get_mapped_range().unwrap();
        for (pixel, color) in [(2, expected[0]), (6, expected[1])] {
            assert_eq!(
                &data[pixel * 4..pixel * 4 + 3],
                &color,
                "properties={properties:?}, pixel={pixel}"
            );
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

fn frame<'a>(
    hud: &'a crate::hud::HudBatch,
    celestial: &'a [super::super::Vertex],
) -> super::super::Frame<'a> {
    super::super::Frame {
        camera: Default::default(),
        yaw: Default::default(),
        pitch: Default::default(),
        fov_degrees: Default::default(),
        eye: Default::default(),
        eye_rotation: Default::default(),
        first_person: Default::default(),
        bob: Mat4::IDENTITY,
        spin: Mat4::IDENTITY,
        sky: Default::default(),
        shadow_scene_revision: Default::default(),
        shadow_cache: Default::default(),
        distance: Default::default(),
        detail_distance: 128.0,
        depth_far: 1024.0,
        underwater: Default::default(),
        water_plane: Default::default(),
        cave_culling: Default::default(),
        title: Default::default(),
        sky_blocked: Default::default(),
        skybox: true,
        end_sky: Default::default(),
        end_sky_sprite: Default::default(),
        boss_world_fog: Default::default(),
        environment_fog: Default::default(),
        water_fog: Default::default(),
        fog: Default::default(),
        dark_disc: Default::default(),
        outline: Default::default(),
        outline_after_terrain: Default::default(),
        outline_high_contrast: Default::default(),
        pvp: Default::default(),
        debug_lines: Default::default(),
        debug_lines_on_top: Default::default(),
        debug_crosshair: Default::default(),
        hud,
        glint_alpha: Default::default(),
        glint_ticks: Default::default(),
        nether_lighting: Default::default(),
        entities: Default::default(),
        static_block_entities: Default::default(),
        visible_static_block_entity_sections: Default::default(),
        static_block_entity_stats: Default::default(),
        entity_motion: Default::default(),
        entity_motion_posed: Default::default(),
        glowing: Default::default(),
        entity_spans: Default::default(),
        skin: Default::default(),
        weather: Default::default(),
        weather_time: crate::weather::Time::new(0, 0.0),
        dynamic_lights: Default::default(),
        colored_lights: Default::default(),
        emissive_layers: Default::default(),
        clouds: Default::default(),
        lightning: Default::default(),
        additive: Default::default(),
        see_through: Default::default(),
        world_primitives: Default::default(),
        shadow_only: Default::default(),
        celestial,
        stars: Default::default(),
        hand: Default::default(),
        hand_items: Default::default(),
        hand_fov_degrees: Default::default(),
        screen: Default::default(),
        pack: Default::default(),
        border: Default::default(),
        game_time: Default::default(),
        shader_game_time: Default::default(),
        end_portal_sprites: Default::default(),
        crumbling_sprite: Default::default(),
        ticks: Default::default(),
        darkness: Default::default(),
        night_vision: Default::default(),
        block_flicker: Default::default(),
        boss_darken: Default::default(),
        menu_blur: Default::default(),
        chunk_fade: Default::default(),
        post_effects: crate::post_chain::Request {
            effects: &[],
            root: std::path::Path::new("."),
            menu_blur_radius: 0.0,
            globals: Default::default(),
        },
    }
}

#[test]
fn pack_visibility_selects_geometry_without_changing_order_or_unrelated_parts() {
    let hud = crate::hud::HudBatch::new([8, 1]);
    let mut frame = frame(&hud, &[]);
    frame.sky.sunrise_color[3] = 1.0;
    frame.stars = Some((Mat4::IDENTITY, 1.0));
    frame.dark_disc = true;
    for mask in 0u8..16 {
        let visibility = horizon_shaders::SkyVisibility {
            sky: mask & 1 != 0,
            stars: mask & 2 != 0,
            sun: mask & 4 != 0,
            moon: mask & 8 != 0,
        };
        let selected = super::super::pack_sky::parts(
            [20..30, 30..40, 40..50, 50..60, 60..70],
            &frame,
            2,
            visibility,
        );
        let mut expected = Vec::new();
        if mask & 1 != 0 {
            expected.push(("SKY", false, 20..30));
        }
        expected.push(("SUNSET", false, 30..40));
        if mask & 4 != 0 {
            expected.push(("SUN", true, 0..6));
        }
        if mask & 8 != 0 {
            expected.push(("MOON", true, 6..12));
        }
        if mask & 2 != 0 {
            expected.push(("STARS", false, 40..50));
        }
        expected.push(("VOID", false, 50..60));
        assert_eq!(
            selected
                .into_iter()
                .map(|p| (p.stage, p.textured, p.range))
                .collect::<Vec<_>>(),
            expected,
            "mask={mask}"
        );
        frame.end_sky = true;
        frame.boss_world_fog = true;
        let fan = super::super::pack_sky::parts(
            [20..30, 30..40, 40..50, 50..60, 60..70],
            &frame,
            2,
            visibility,
        );
        assert_eq!(fan.len(), 1);
        assert_eq!(fan[0].stage, "SKY");
        assert_eq!(fan[0].range, 60..70);
        frame.end_sky = false;
        frame.boss_world_fog = false;
    }
    frame.sky.sunrise_color[3] = 0.0;
    frame.stars = None;
    frame.dark_disc = false;
    let parts = super::super::pack_sky::parts(
        [20..30, 30..40, 40..50, 50..60, 60..70],
        &frame,
        0,
        Default::default(),
    );
    assert_eq!(
        parts.into_iter().map(|p| p.stage).collect::<Vec<_>>(),
        ["SKY"]
    );
}

#[test]
fn pack_visibility_native_fallback_preserves_end_flashes_and_native_sun_ownership() {
    use super::super::pack_sky::celestial_ranges;
    for mask in 0u8..4 {
        let visibility = horizon_shaders::SkyVisibility {
            sun: mask & 1 != 0,
            moon: mask & 2 != 0,
            ..Default::default()
        };
        for own_sun in [false, true] {
            let expected: Vec<u32> = match (mask, own_sun) {
                (1, false) => (0..6).collect(),
                (2, _) | (3, true) => (6..12).collect(),
                (3, false) => (0..12).collect(),
                _ => Vec::new(),
            };
            assert_eq!(
                celestial_ranges(8, false, own_sun, visibility)
                    .flatten()
                    .collect::<Vec<_>>(),
                expected
            );
            let mut extra = expected;
            extra.extend(12..18);
            assert_eq!(
                celestial_ranges(12, false, own_sun, visibility)
                    .flatten()
                    .collect::<Vec<_>>(),
                extra
            );
            assert!(
                celestial_ranges(12, true, own_sun, visibility)
                    .flatten()
                    .eq(0..18)
            );
            assert!(
                celestial_ranges(0, false, own_sun, visibility)
                    .flatten()
                    .next()
                    .is_none()
            );
        }
    }
}
