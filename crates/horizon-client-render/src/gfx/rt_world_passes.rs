//! RT lighting's passes between the world and the tone mapping (gfx.rs may not grow).

use super::*;

impl Renderer {
    /// RT lighting's passes on the world drawn, before the tone mapping (`draw_frame`): the
    /// volumetric clouds, the froxels' medium, the TAA's resolve, the rain's drops and the
    /// snow's flakes, and the heat haze above the flames.
    pub(super) fn after_world_rt(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        laps: &mut crate::spans::Laps,
        frame: &Frame,
        rt: &shadow_rays::Rays,
        hw: Option<&hw_rays::Bindings>,
        world_view: &wgpu::TextureView,
        world_depth: &wgpu::TextureView,
        world_size: [u32; 2],
        view_proj: Mat4,
        clean_view_proj: Mat4,
        rt_view: usize,
        rt_sun: Vec3,
        rt_sun_light: [f32; 4],
        rain_level: f32,
        [rt_clouds_on, froxels_now, taa_now]: [bool; 3],
        [rain_drops, snow_flakes, pack_draws_weather, world_linear]: [bool; 4],
    ) {
        // RT lighting's volumetric clouds, over the world's sky before the tone mapping, bent by
        // the Earth's curvature as the distant terrain.
        let earth_bend = if self.earth_curvature {
            (0.5 / lod::EARTH_RADIUS) as f32
        } else {
            0.0
        };
        if rt_clouds_on && let Some(input) = &frame.clouds {
            // The light is the sun's, or the moon's (opposite it) once the sun has set.
            let to_light = if rt_sun.y > 0.0 { rt_sun } else { -rt_sun };
            let light = rt_sun_light;
            let seconds = (input.time as f64 + input.partial_tick) / 20.0;
            if rt_clouds::mode() == rt_clouds::Mode::Mesh {
                let mesh = self
                    .rt_cloud_mesh
                    .get_or_insert_with(|| rt_cloud_mesh::RtCloudMesh::new(&self.device));
                mesh.update(&self.device, frame.camera, seconds);
                mesh.draw(
                    &self.device,
                    &mut self.uploads,
                    encoder,
                    world_view,
                    world_depth,
                    view_proj,
                    frame.camera,
                    &rt_cloud_mesh::Light {
                        direction: to_light,
                        color: [light[0], light[1], light[2]],
                        sun: rt_sun,
                        curvature: earth_bend,
                    },
                    &self.atmosphere.view,
                );
                gpu_passes::stamp(encoder, "clouds: mesh");
            }
        }
        if rt_clouds_on
            && matches!(
                rt_clouds::mode(),
                rt_clouds::Mode::March | rt_clouds::Mode::Dome
            )
            && let Some(input) = &frame.clouds
            && self.rt_clouds_ready()
            && let Some(clouds) = self.rt_clouds.as_mut()
        {
            let to_light = if rt_sun.y > 0.0 { rt_sun } else { -rt_sun };
            let light = rt_sun_light;
            let params = rt_clouds::Params {
                light: to_light,
                light_color: [light[0], light[1], light[2]],
                sun: rt_sun,
                time: (input.time as f64 + input.partial_tick) / 20.0,
                thick: rain_level.clamp(0.0, 1.0),
                curvature: earth_bend,
                distance: self.cloud_distance,
            };
            // Their shadows, for the terrain the next frame (and the other eyes this one).
            if self.eye.is_none_or(|e| e.view_index <= 1) {
                clouds.update_shadows(
                    &self.device,
                    &mut self.uploads,
                    encoder,
                    world_depth,
                    frame.camera,
                    &params,
                    &self.atmosphere.view,
                    &mut self.cloud_shadows,
                );
            }
            if rt_clouds::mode() == rt_clouds::Mode::Dome && rt_clouds::dome_sees(frame.camera) {
                // One march a frame for every view (both eyes see clouds alike): the first view
                // updates the dome, each view looks it up.
                if self.eye.is_none_or(|e| e.view_index <= 1) {
                    clouds.update_dome(
                        &self.device,
                        &mut self.uploads,
                        encoder,
                        world_depth,
                        frame.camera,
                        &params,
                        &self.atmosphere.view,
                    );
                    gpu_passes::stamp(encoder, "clouds: dome march");
                }
                clouds.draw_dome(
                    &self.device,
                    &mut self.uploads,
                    encoder,
                    world_depth,
                    world_view,
                    world_size,
                    view_proj,
                );
                gpu_passes::stamp(encoder, "clouds: dome");
            } else {
                clouds.draw(
                    &self.device,
                    &mut self.uploads,
                    encoder,
                    rt_view,
                    world_depth,
                    world_view,
                    world_size,
                    view_proj,
                    frame.camera,
                    &params,
                    &self.atmosphere.view,
                );
                gpu_passes::stamp(encoder, "clouds: march");
            }
        }
        laps.lap("rp_clouds");
        // Froxel volumetrics (froxels.rs, hook 7): this view's medium on the world drawn, before
        // the tone mapping. `Frame::underwater` says the water; the sky light and the weather come
        // from `set_volumetrics` (`frame.pack`'s are only filled with a shader pack picked).
        if froxels_now
            && let (Some(grid), Some(lights)) = (self.voxels.views(), self.voxels.light_buffers())
        {
            let mut camera = self.froxels.camera;
            camera.in_water |= frame.underwater;
            camera.rain = camera.rain.max(frame.pack.rain);
            camera.thunder = camera.thunder.max(frame.pack.thunder);
            let saved = std::mem::replace(&mut self.froxels.camera, camera);
            self.froxels.run(
                &self.device,
                &mut self.uploads,
                encoder,
                froxels::Inputs {
                    view: rt_view,
                    depth: world_depth,
                    color: world_view,
                    color_format: self.world_format,
                    size: world_size,
                    view_proj,
                    camera: frame.camera,
                    rays: rt,
                    grid,
                    lights,
                    sky: &self.atmosphere.view,
                    hardware: hw.map(hw_rays::Bindings::hardware),
                    // The second eye reads the first's froxels.
                    share: (rt_view == 2).then_some(1),
                },
            );
            self.froxels.camera = saved;
            gpu_passes::stamp(encoder, "froxels: apply");
        }
        laps.lap("rp_froxels");
        // Temporal anti-aliasing (taa.rs): the world and its media resolved against the history,
        // before the tone mapping's exposure and bloom read them (the hand and the HUD come after).
        if taa_now {
            self.taa.resolve(
                &self.device,
                &mut self.uploads,
                encoder,
                taa::Inputs {
                    view: rt_view,
                    color: world_view,
                    depth: world_depth,
                    size: world_size,
                    view_proj: clean_view_proj,
                    camera: frame.camera,
                    copy_back: true,
                },
            );
            gpu_passes::stamp(encoder, "taa");
        }
        // RT lighting's physical rain (`rain`): the drops over the resolved scene, depth tested
        // against the world's. Before the resolve the TAA's history (a sixteenth new a frame) would
        // wash out thin streaks that move every frame; they take the still matrix, as the rays do.
        // The snow's flakes (`snow`) in the same pass.
        if (rain_drops || snow_flakes) && !pack_draws_weather {
            let view_proj_offset = std::mem::offset_of!(Globals, view_proj) as u64;
            if taa_now {
                self.uploads.buffer(
                    encoder,
                    &self.globals,
                    view_proj_offset,
                    bytemuck::cast_slice(&clean_view_proj.to_cols_array()),
                );
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Rain drops"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: world_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: world_depth,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                if rain_drops {
                    self.rain.draw(&mut pass, &self.group);
                }
                if snow_flakes {
                    self.snow.draw(&mut pass, &self.group);
                }
            }
            if taa_now {
                self.uploads.buffer(
                    encoder,
                    &self.globals,
                    view_proj_offset,
                    bytemuck::cast_slice(&view_proj.to_cols_array()),
                );
            }
            gpu_passes::stamp(encoder, "rain drops");
        }
        laps.lap("rp_taa_rain");
        // Heat haze above the flames in view (haze.rs), on the resolved scene; nothing is recorded
        // with no flame in range.
        if rt.lighting && world_linear && self.world_format == tonemap::SCENE_FORMAT {
            let flames = self.voxels.flames(frame.camera, haze::RANGE);
            self.haze.run(
                &self.device,
                &mut self.uploads,
                encoder,
                haze::Inputs {
                    view: rt_view,
                    color: world_view,
                    depth: world_depth,
                    size: world_size,
                    view_proj: clean_view_proj,
                    camera: frame.camera,
                    time: self.water_clock.elapsed().as_secs_f32() % 3600.0,
                    flames: &flames,
                },
            );
        } else {
            self.haze.release();
        }
        gpu_passes::stamp(encoder, "heat haze");
    }
}
