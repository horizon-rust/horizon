//! RT lighting's passes before the world: the hardware structures' update (`hw_update`), the
//! depth prepass the rays start from (one view, both eyes, the translucent cover), the terrain
//! groups they draw, and the frame's ray-traced shadow inputs (gfx.rs may not grow).

use super::*;

impl Renderer {
    /// Whether the volumetric clouds are made: the first call starts making them on a thread of
    /// their own, and the frames until they are drawn without them.
    pub(super) fn rt_clouds_ready(&mut self) -> bool {
        if self.rt_clouds.is_none() {
            match &self.rt_clouds_making {
                None => {
                    let (sent, making) = std::sync::mpsc::channel();
                    let (device, queue) = (self.device.clone(), self.queue.clone());
                    // `HORIZON_RT_SKIP` bit 524288: made on the frame, as before (to compare).
                    let inline = std::env::var("HORIZON_RT_SKIP")
                        .ok()
                        .and_then(|v| v.parse::<u32>().ok())
                        .is_some_and(|skip| skip & 524_288 != 0);
                    if inline {
                        self.rt_clouds = Some(rt_clouds::RtClouds::new(&self.device, &self.queue));
                        return true;
                    }
                    let spawned = std::thread::Builder::new()
                        .name("Horizon clouds".into())
                        .spawn(move || {
                            let _ = sent.send(rt_clouds::RtClouds::new(&device, &queue));
                        });
                    match spawned {
                        Ok(_) => self.rt_clouds_making = Some(making),
                        // No thread: made here, as before.
                        Err(_) => {
                            self.rt_clouds =
                                Some(rt_clouds::RtClouds::new(&self.device, &self.queue))
                        }
                    }
                }
                Some(making) => match making.try_recv() {
                    Ok(clouds) => {
                        self.rt_clouds = Some(clouds);
                        self.rt_clouds_making = None;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    // The thread died: made here.
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        self.rt_clouds_making = None;
                        self.rt_clouds = Some(rt_clouds::RtClouds::new(&self.device, &self.queue));
                    }
                },
            }
        }
        self.rt_clouds.is_some()
    }
    /// RT lighting's hybrid: structures for the sections near `camera` that lack one (a frame's
    /// budget of them), and the frame's top-level structure.
    pub(super) fn hw_update(&mut self, encoder: &mut wgpu::CommandEncoder, camera: DVec3) {
        let Some(hw) = self.hw_rays.as_mut() else {
            return;
        };
        let center = (
            (camera.x.floor() as i32) >> 4,
            (camera.z.floor() as i32) >> 4,
        );
        hw.micromaps.set_center(center);
        // The sections lacking a structure, the nearest the camera first: a frame's budget of
        // them (arriving in a new place, the far ring waits a few frames, not the ground underfoot).
        let mut candidates = Vec::new();
        for (key, s, _) in self.sections.iter() {
            if s.solid + s.translucent > 0 && hw_rays::HwRays::near(center, key) && !hw.has(key) {
                let (dx, dz) = (key.0 - center.0, key.2 - center.1);
                candidates.push((
                    dx * dx + dz * dz,
                    hw_rays::ToBuild {
                        key,
                        page: s.page,
                        start: s.start,
                        quads: s.solid,
                        // Its opaque quads and its cutouts, a geometry each run (`hw_rays`).
                        runs: s.sides.geometries(),
                        translucent: s.translucent,
                    },
                ));
            }
        }
        if candidates.len() > hw.budget() {
            candidates.select_nth_unstable_by_key(hw.budget(), |c| c.0);
            candidates.truncate(hw.budget());
        }
        let todo: Vec<_> = candidates.into_iter().map(|c| c.1).collect();
        let arena = &self.arena;
        hw.update(
            &self.device,
            &self.queue,
            encoder,
            &todo,
            |page| arena.buffer(page).clone(),
            &self.indices,
            center,
            camera,
        );
    }
    /// RT lighting's hybrid: the moving geometry of this frame in the rays (`hw_rays::HwRays::finish`),
    /// once the entities' vertices are known: the entities, the view model (with Remix's
    /// perspective correction for its own field of view) and the mesh clouds' tiles.
    pub(super) fn hw_finish(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        frame: &Frame,
        solid_quads: u32,
        posed: u32,
        clouds: bool,
    ) {
        // The view model is drawn in the window's view only. And not beside the player's own body
        // (`Frame::rt_body`): the body has its arms and its head where they are, and the view
        // model's arm and held block, drawn large beside the eye, threw a second shadow next to
        // the body's, a block the size of a head by its shoulder. `HORIZON_RT_HAND_SHADOW=1`
        // keeps the view model in the rays with the body.
        let hand = if self.eye.is_some() || (!frame.rt_body.is_empty() && !rt_hand_shadow()) {
            &[]
        } else {
            frame.hand
        };
        let Some(hw) = self.hw_rays.as_mut() else {
            return;
        };
        let solid = self
            .entity_batch
            .combined
            .len()
            .min(solid_quads as usize * 4)
            .min(self.entity_batch.before_particles * 4);
        let ratio = (frame.hand_fov_degrees.to_radians() / 2.0).tan()
            / (frame.fov_degrees.to_radians() / 2.0).tan();
        let view = crate::camera::view_matrix(frame.yaw, frame.pitch);
        let hand_to_world =
            view.inverse() * Mat4::from_scale(Vec3::new(ratio, ratio, 1.0)) * frame.bob * view;
        let tiles = match (&self.rt_cloud_mesh, clouds) {
            (Some(mesh), true) => mesh.tiles().collect(),
            _ => Vec::new(),
        };
        hw.finish(
            &self.device,
            &self.queue,
            encoder,
            &hw_rays::Dynamic {
                posed: (posed > 0).then_some((&self.entity_buffer, posed)),
                entities: &self.entity_batch.combined[..solid],
                body: frame.rt_body,
                hand,
                hand_to_world,
                clouds: tiles,
            },
            &self.indices,
            frame.camera,
        );
    }
    /// What the shadow rays read of the hybrid's (`shadow_rays::Hardware`): the top-level
    /// structure, the arena's first pages, the first sprite array, and the moving geometry's.
    pub(super) fn hw_bindings(&self) -> Option<hw_rays::Bindings> {
        let hw = self.hw_rays.as_ref()?;
        let atlas = self
            .hud
            .atlas_texture
            .as_ref()?
            .create_view(&Default::default());
        let (_, first) = self.arena.page_buffers().find(|(p, _)| *p == 0)?;
        let arena = std::array::from_fn(|i| {
            let page = self.arena.page_buffers().find(|&(p, _)| p as usize == i);
            page.map_or(first, |(_, b)| b).clone()
        });
        let sprites = self
            .sprite_textures
            .first()?
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
        Some(hw_rays::Bindings {
            tlas: hw.tlas.clone(),
            arena,
            sprites,
            instances: hw.instances.clone(),
            dynamics: hw.dynamics.clone(),
            atlas,
            wind: self.wind.buffer.clone(),
        })
    }
    /// RT lighting's rays on this frame: the solid terrain's `draws` into a depth of the world
    /// target's `size`, then the rays' pass on it; returns the mask and the light sources' light
    /// the terrain reads.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn rt_prepass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        view_proj: Mat4,
        camera: DVec3,
        rt: &shadow_rays::Rays,
        phase1: u32,
        draws: u32,
        translucent: [u32; 2],
    ) -> [wgpu::TextureView; 6] {
        let foveation = match &self.eye {
            Some(eye) => eye.foveation,
            None => shadow_rays::Foveation::screen(size),
        };
        // VR's prepass at the rays' resolution (`half_prepass`): a depth pixel a mask texel.
        let (size, foveation) = match self.eye {
            Some(_) if foveation.scale == 2 && half_prepass() => (
                size.map(|v| v.div_ceil(2)),
                shadow_rays::Foveation {
                    scale: 1,
                    ..foveation
                },
            ),
            _ => (size, foveation),
        };
        let (depth, first_depth, cover) =
            match self.eye_prepass(encoder, size, view_proj, phase1, draws, translucent) {
                Some(d) => d,
                None => {
                    let (depth, cover) =
                        self.view_prepass(encoder, size, phase1, draws, translucent);
                    (depth, None, cover)
                }
            };
        let view = self.eye.map_or(0, |e| e.view_index);
        let grid = self.voxels.views().expect("the grid runs");
        let lights = self.voxels.light_buffers().expect("the grid runs");
        self.far_heights.prepare(
            &self.device,
            &self.queue,
            encoder,
            [rt.light[0], rt.light[1], rt.light[2]],
        );
        let hw = self.hw_bindings();
        // The RTX 4090 build's sun pass (horizon-rtx `sun`): recorded by the kernel's run once
        // its uniform is written, with the structure built raw and the hybrid's bindings.
        #[cfg(feature = "rtx")]
        let pass_bits = self.sun_pass.as_ref().map_or(0, |p| p.kinds().bits());
        #[cfg(feature = "rtx")]
        let mut sun_closure = match (
            &mut self.sun_pass,
            self.hw_rays.as_ref().and_then(hw_rays::HwRays::raw),
            hw.as_ref(),
        ) {
            (Some(pass), Some(tlas), Some(bindings)) if rt.lighting => {
                let device = &self.device;
                let depth = &depth;
                let split = &mut self.split_commands;
                Some(
                    move |encoder: &mut wgpu::CommandEncoder,
                          uniform: &wgpu::Buffer,
                          mask_size: [u32; 2],
                          reads: &shadow_rays::PassReads<'_>| {
                        let views = pass.run(
                            device,
                            encoder,
                            split,
                            horizon_rtx::SunInputs {
                                view,
                                uniform,
                                depth,
                                mask_size,
                                tlas,
                                arena: &bindings.arena,
                                sprites: &bindings.sprites,
                                instances: &bindings.instances,
                                dynamics: &bindings.dynamics,
                                atlas: &bindings.atlas,
                                sprite_split: sprite_split(),
                                kernel: horizon_rtx::KernelReads {
                                    sky: reads.sky,
                                    last_mask: reads.last_mask,
                                    last_light: reads.last_light,
                                    last_direct: reads.last_direct,
                                    last_gi: reads.last_gi,
                                    cache: reads.cache,
                                },
                            },
                        );
                        (views.sun, views.gi)
                    },
                )
            }
            _ => None,
        };
        #[cfg(feature = "rtx")]
        let sun_pass = sun_closure
            .as_mut()
            .map(|f| (pass_bits, f as &mut shadow_rays::PipelinePass<'_>));
        #[cfg(not(feature = "rtx"))]
        let sun_pass = None;
        let [mask, blocks, sun, reflection, refraction] = self.shadow_rays.run(
            &self.device,
            &mut self.uploads,
            encoder,
            view,
            &depth,
            size,
            view_proj,
            camera,
            rt,
            foveation,
            grid,
            lights,
            self.voxels.light_visibility().expect("the grid runs"),
            &self.atmosphere.view,
            self.far_heights.views(),
            hw.as_ref().map(hw_rays::Bindings::hardware),
            first_depth.as_ref(),
            cover.as_ref(),
            sun_pass,
        );
        [mask, blocks, sun, reflection, refraction, depth]
    }
    /// VR's two eyes: the first draws both eyes' prepass in one multiview pass and its own
    /// layer; the second takes its layer, and the first's for its rays to take the first eye's
    /// light (`shadow_rays::ShadowRays::run`). `None` elsewhere, or without multiview.
    pub(super) fn eye_prepass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        view_proj: Mat4,
        phase1: u32,
        draws: u32,
        translucent: [u32; 2],
    ) -> Option<(
        wgpu::TextureView,
        Option<wgpu::TextureView>,
        Option<wgpu::TextureView>,
    )> {
        let eye = self.eye?;
        let mut depths = self.eye_depths.take()?;
        let Some(next) = eye.next_view_proj else {
            let layer = depths.next.take();
            let cover = depths.next_cover.take();
            let first = depths
                .layers
                .as_ref()
                .map(|(_, _, [first, _])| first.clone());
            self.eye_depths = Some(depths);
            return layer.map(|layer| (layer, first, cover));
        };
        if depths.layers.as_ref().is_none_or(|(s, _, _)| *s != size) {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Both eyes' terrain depth for the rays"),
                size: wgpu::Extent3d {
                    width: size[0],
                    height: size[1],
                    depth_or_array_layers: 2,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let layer = |i| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: i,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            };
            let both = texture.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
            depths.layers = Some((size, both, [layer(0), layer(1)]));
        }
        let (_, both, [first, second]) = depths.layers.clone().expect("made above");
        self.uploads.buffer(
            encoder,
            &depths.matrices,
            0,
            bytemuck::cast_slice(&[view_proj.to_cols_array(), next.to_cols_array()]),
        );
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Both eyes' terrain depth for the rays"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &both,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                multiview_mask: std::num::NonZeroU32::new(0b11),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.group, &[]);
            pass.set_bind_group(1, &depths.group, &[]);
            self.prepass_draws(&mut pass, &depths.opaque, &depths.pipeline, phase1, draws);
        }
        gpu_passes::stamp(encoder, "rays: depth prepass (both eyes)");
        depths.next = Some(second);
        let cover = psr_cover().then(|| {
            if depths.covers.as_ref().is_none_or(|(s, _, _)| *s != size) {
                let texture = cover_texture(&self.device, size, 2);
                let layer = |i| {
                    texture.create_view(&wgpu::TextureViewDescriptor {
                        dimension: Some(wgpu::TextureViewDimension::D2),
                        base_array_layer: i,
                        array_layer_count: Some(1),
                        ..Default::default()
                    })
                };
                let all = texture.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    ..Default::default()
                });
                depths.covers = Some((size, all, [layer(0), layer(1)]));
            }
            let (_, all, [this_eye, next_eye]) = depths.covers.clone().expect("made above");
            self.draw_cover(
                encoder,
                &all,
                &both,
                &depths.cover,
                Some(&depths.group),
                std::num::NonZeroU32::new(0b11),
                translucent,
            );
            gpu_passes::stamp(encoder, "rays: translucent cover (both eyes)");
            depths.next_cover = Some(next_eye);
            this_eye
        });
        self.eye_depths = Some(depths);
        Some((first, None, cover))
    }
    /// The translucent terrain's cover over the rays' prepass: the `translucent` draws (the
    /// frame's first and count) into `cover` (cleared), tested against `depth` without writing
    /// it, 1 where water or glass lies in front of the solid terrain, so the rays' mirrors and
    /// refraction look for them only there (shadow_rays_hw.wgsl `hw_reflect`). `views` is the
    /// eyes' matrices' group with `multiview`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn draw_cover(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        cover: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        pipeline: &wgpu::RenderPipeline,
        views: Option<&wgpu::BindGroup>,
        multiview: Option<std::num::NonZeroU32>,
        [first, count]: [u32; 2],
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Translucent terrain's cover for the rays"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: cover,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            multiview_mask: multiview,
            ..Default::default()
        });
        if count == 0 {
            return;
        }
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, &self.group, &[]);
        if let Some(views) = views {
            pass.set_bind_group(1, views, &[]);
        }
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_vertex_buffer(1, self.instances.slice(..));
        self.draw_sections(&mut pass, &self.indirect, 0, first, count);
    }
    /// The prepass's draws (`prepass_draws`) from the frame's solid draws: each section's runs
    /// split into the quads whose sprites have no transparent texel, drawn first without a
    /// fragment shader (the depth written at the rasterizer's rate, no texture read), and the
    /// cutouts after them with the alpha test (`Sides::split`). Distant terrain tiles keep the
    /// test: their quads are not sorted by kind, and a fading tile dithers in the shader. Each
    /// list keeps the draws' order, front to back by band and page.
    pub(super) fn split_prepass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        visible: &[resident::Visible],
        solid_draws: u32,
    ) {
        let mut cutouts = std::mem::take(&mut self.prepass_split);
        let mut cutout_pages = std::mem::take(&mut self.prepass_pages);
        cutouts.clear();
        cutout_pages.clear();
        let mut opaque = Vec::with_capacity(solid_draws as usize);
        let mut opaque_pages = Vec::with_capacity(solid_draws as usize);
        for (d, &page) in self.draws[..solid_draws as usize]
            .iter()
            .zip(&self.draw_pages[..solid_draws as usize])
        {
            let instance = d[4];
            let Some(section) = visible
                .get(instance as usize)
                .map(|v| self.sections.get(v.slot))
            else {
                cutouts.push([d[0], 1, d[2], d[3], instance]);
                cutout_pages.push(page);
                continue;
            };
            let start = section.start as u32;
            let first = d[3].wrapping_sub(start) / 4;
            section.sides.split(first, d[0] / 6, |first, quads, kind| {
                let draw = [quads * 6, 1, 0, start + first * 4, instance];
                if kind {
                    opaque.push(draw);
                    opaque_pages.push(page);
                } else {
                    cutouts.push(draw);
                    cutout_pages.push(page);
                }
            });
        }
        let counts = [opaque.len() as u32, cutouts.len() as u32];
        opaque.append(&mut cutouts);
        opaque_pages.append(&mut cutout_pages);
        self.prepass_split = opaque;
        self.prepass_pages = opaque_pages;
        if self.multi_draw && !self.prepass_split.is_empty() {
            let len = self.prepass_split.len() as u64;
            if len > self.prepass_capacity {
                self.prepass_capacity = len.next_power_of_two();
                self.prepass_indirect = self.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Prepass draws"),
                    size: self.prepass_capacity * 20,
                    usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
            }
            self.uploads.buffer(
                encoder,
                &self.prepass_indirect,
                0,
                bytemuck::cast_slice(&self.prepass_split),
            );
        }
        self.prepass_counts = Some(counts);
    }
    /// One view's prepass into its size's depth.
    pub(super) fn view_prepass(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        size: [u32; 2],
        phase1: u32,
        draws: u32,
        translucent: [u32; 2],
    ) -> (wgpu::TextureView, Option<wgpu::TextureView>) {
        let depth = match self.rt_depths.iter().find(|(s, _)| *s == size) {
            Some((_, d)) => d.clone(),
            None => {
                if self.rt_depths.len() >= 5 {
                    self.rt_depths.remove(0);
                }
                let mut config = self.config.clone();
                config.width = size[0];
                config.height = size[1];
                let d = Self::depth(&self.device, &config);
                self.rt_depths.push((size, d.clone()));
                d
            }
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Terrain depth for the rays"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            pass.set_bind_group(0, &self.group, &[]);
            self.prepass_draws(
                &mut pass,
                &self.opaque_depth,
                &self.solid_depth,
                phase1,
                draws,
            );
        }
        gpu_passes::stamp(encoder, "rays: depth prepass");
        let cover = psr_cover().then(|| {
            let cover = match self.rt_covers.iter().find(|(s, _)| *s == size) {
                Some((_, c)) => c.clone(),
                None => {
                    if self.rt_covers.len() >= 5 {
                        self.rt_covers.remove(0);
                    }
                    let c = cover_texture(&self.device, size, 1).create_view(&Default::default());
                    self.rt_covers.push((size, c.clone()));
                    c
                }
            };
            self.draw_cover(
                encoder,
                &cover,
                &depth,
                &self.rt_cover,
                None,
                None,
                translucent,
            );
            gpu_passes::stamp(encoder, "rays: translucent cover");
            cover
        });
        (depth, cover)
    }
    /// The prepass's draws: every solid section, both phases of occlusion culling; the opaque
    /// runs with `opaque` (no fragment shader) where they were split from the cutouts
    /// (`split_prepass`), the rest with `alpha_test`.
    pub(super) fn prepass_draws(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        opaque: &wgpu::RenderPipeline,
        alpha_test: &wgpu::RenderPipeline,
        phase1: u32,
        draws: u32,
    ) {
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.set_vertex_buffer(1, self.instances.slice(..));
        if let Some([opaque_runs, cutout_runs]) = self.prepass_counts {
            let lists = (&self.prepass_split[..], &self.prepass_pages[..]);
            let indirect = &self.prepass_indirect;
            pass.set_pipeline(opaque);
            self.draw_list(
                pass,
                lists,
                indirect,
                (0, 0, opaque_runs),
                false,
                self.multi_draw,
            );
            pass.set_pipeline(alpha_test);
            let range = (0, opaque_runs, cutout_runs);
            self.draw_list(pass, lists, indirect, range, false, self.multi_draw);
            return;
        }
        pass.set_pipeline(alpha_test);
        // The sections occlusion culling left to its second phase have no instance yet in the
        // indirect arguments (the GPU's test after the world's first phase gives them one):
        // the rays see them all, from the draws' own counts, or a section meshed again (no
        // visibility history) would be missing for a frame, the room behind it lit as if open.
        self.draw_sections(pass, &self.indirect, 0, 0, phase1);
        self.draw_sections_with(pass, &self.indirect, 0, phase1, draws - phase1, false);
    }
    /// The terrain's groups, with `history` (RT lighting's last light for the view drawn, or this
    /// frame's), `blocks` (this frame's light from the light sources) and `scene` (the view's last
    /// resolved frame, for the screen-space reflections).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn terrain_groups_with(
        &mut self,
        history: Option<&wgpu::TextureView>,
        blocks: Option<&wgpu::TextureView>,
        sun: Option<&wgpu::TextureView>,
        depth: Option<&wgpu::TextureView>,
        reflection: Option<&wgpu::TextureView>,
        refraction: Option<&wgpu::TextureView>,
        scene: Option<&wgpu::TextureView>,
    ) {
        let history = history.unwrap_or(&self.rt_history_unused);
        let blocks = blocks.unwrap_or(&self.rt_history_unused);
        let sun = sun.unwrap_or(&self.rt_sun_unused);
        let depth = depth.unwrap_or(&self.rt_depth_unused);
        let reflection = reflection.unwrap_or(&self.rt_history_unused);
        let refraction = refraction.unwrap_or(&self.rt_history_unused);
        let scene = scene.unwrap_or(&self.rt_history_unused);
        // The terrain, still fluids, native cutout models and the debug crosshair: the same
        // resources, each with its own globals or sampler.
        let group = |label: &str, globals: &wgpu::Buffer, sampler: &wgpu::Sampler| {
            terrain_group(
                &self.device,
                label,
                &self.terrain_layout,
                globals,
                &self.sprite_views,
                sampler,
                &self.lightmap_view,
                &self.lightmap_sampler,
                &self.emission_stats.view,
                history,
                blocks,
                &self.atmosphere.view,
                sun,
                depth,
                reflection,
                refraction,
                &self.moon_map,
                &self.milky_way_map,
                &self.aldhani,
                self.rain.map(),
                scene,
                &self.wind.buffer,
                &self.cloud_shadows,
            )
        };
        let terrain = group(
            "Horizon terrain resources",
            &self.globals,
            &self.sprite_sampler,
        );
        let fluid = group("Still fluid resources", &self.globals, &self.fluid_sampler);
        let model = group(
            "Native cutout model resources",
            &self.globals,
            &self.terrain_model_sampler,
        );
        let crosshair = group(
            "Debug crosshair resources",
            &self.crosshair_globals,
            &self.sprite_sampler,
        );
        self.group = terrain;
        self.fluid_group = fluid;
        self.terrain_model_group = model;
        self.crosshair_group = crosshair;
    }
    /// This frame's ray-traced shadows: the grid written up to date, the light's direction (the
    /// sun by day, the moon by night, on the tilted path) and how strong its shadows are.
    pub(super) fn ray_traced_shadows(&mut self, frame: &Frame) -> shadow_rays::Rays {
        let Some([x0, z0, top]) =
            crate::spans::time("rt_grid", || self.voxels.update(&self.queue, frame.camera))
        else {
            return shadow_rays::Rays::default();
        };
        let sun = crate::celestial::direction(frame.sky.sun_angle, frame.sun_path_tilt);
        let moon = crate::celestial::direction(frame.sky.moon_angle, frame.sun_path_tilt);
        // RT lighting's real sun: its light (`atmosphere`) fades as it sets on its own, the moon's
        // is taken as it rises; ray-traced shadows over vanilla's light fade over 11 degrees.
        let (light, strength) = match (sun.y > 0.0, self.rt_lighting) {
            (true, true) => (sun, (sun.y / 0.01).clamp(0.0, 1.0)),
            (false, true) => (moon, (moon.y / 0.01).clamp(0.0, 1.0)),
            (true, false) => (sun, (sun.y / 0.2).clamp(0.0, 1.0)),
            (false, false) => (moon, (moon.y / 0.2).clamp(0.0, 1.0) * 0.6),
        };
        let has_sky = (self.rt_shadows || self.rt_lighting)
            && !(frame.title || frame.end_sky || frame.nether_lighting);
        let block = frame.camera.floor();
        let fraction = (frame.camera - block).as_vec3();
        let far = self
            .far_heights
            .update(&self.device, &self.queue, frame.camera, &self.voxels);
        self.wind
            .update(&self.queue, self.far_heights.level(wind::LEVEL));
        shadow_rays::Rays {
            far,
            light: [
                light.x,
                light.y,
                light.z,
                if has_sky { strength } else { 0.0 },
            ],
            camera: [block.x as f32, block.y as f32, block.z as f32, top as f32],
            fraction: [fraction.x, fraction.y, fraction.z, 0.0],
            grid: [x0 as f32, z0 as f32, voxels::WIDTH as f32, rt_steps()],
            ambient: if self.rt_ambient && !frame.title {
                RT_AMBIENT
            } else {
                0.0
            },
            shadows: self.rt_shadows,
            lighting: self.rt_lighting && !frame.title,
            sun: [sun.x, sun.y, sun.z, f32::from(u8::from(self.rt_lighting))],
            // Set with the atmosphere's light in `draw_frame`.
            sun_light: [0.0; 4],
            water: [
                self.water_layers[0],
                self.water_layers[1],
                self.water_clock.elapsed().as_secs_f32() % 3600.0,
                self.lava_layers,
            ],
            glass: self.glass_layers,
            ice: self.ice_layers + 1.0,
            // Set after the hybrid's structures are made this frame (`hw_update`).
            hw_offset: [0.0; 4],
            moving: {
                let mut lights = [[0.0; 4]; shadow_rays::MOVING_LIGHTS];
                for (to, from) in lights.iter_mut().zip(frame.dynamic_lights) {
                    *to = *from;
                }
                lights
            },
            moving_count: frame.dynamic_lights.len().min(shadow_rays::MOVING_LIGHTS),
            // Set when the Eye of Aldhani's field is made this frame (`draw_frame`).
            aldhani: [-1.0, 0.0],
            // Set with the globals (`draw_frame`).
            rain: [0.0; 4],
            rain_sun: [0.0; 4],
            ambient_light: self.rt_ambient_light,
        }
    }
}
