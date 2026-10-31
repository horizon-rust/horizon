//! RT lighting's physical rain (`crate::rain`): the rain map on the GPU (terrain.wgsl's
//! `rain_map`, read by the drops, the wet surfaces and the puddles, and by the rays' reflections in
//! the puddles) and the drops' draw.
//!
//! The drops are instanced streaks with no vertex data (terrain.wgsl `vs_rain`): each instance is
//! a drop's track in a cell of a grid around the camera (`NEAR_SIDE`² one-block cells, then
//! `FAR_SIDE`² four-block ones out to 64 blocks), `TRACKS` (`FAR_TRACKS`) a cell, its size, speed and phase hashed
//! from the cell's world coordinates, so both eyes of a headset see the same drops. A drop falls
//! at its terminal velocity, leaning with the wind, stops on the rain heightmap and splashes there.
//! They draw over the scene after the TAA's resolve (whose history would wash out thin streaks that
//! move every frame), blended (premultiplied), depth tested against the world's and not written.

/// The grids' sides in cells, and the tracks a cell (terrain.wgsl's `RAIN_*` constants).
pub const NEAR_SIDE: u32 = 64;
pub const FAR_SIDE: u32 = 32;
pub const TRACKS: u32 = 24;
pub const FAR_TRACKS: u32 = 16;
/// Instances a frame: every track of both grids.
pub const INSTANCES: u32 = NEAR_SIDE * NEAR_SIDE * TRACKS + FAR_SIDE * FAR_SIDE * FAR_TRACKS;

pub(super) struct Rain {
    map: wgpu::Buffer,
    pipeline: wgpu::RenderPipeline,
}
impl Rain {
    /// `layout` and `shader` are the terrain's (group 0 holds the map, binding 23).
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
    ) -> Self {
        let side = crate::rain::MAP as u64;
        let map = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Rain map"),
            size: side * side * 4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Rain drops"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_rain"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_rain"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    // The scene's alpha is the terrain's sun share (tonemap.wgsl): left alone.
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self { map, pipeline }
    }
    /// The rain map's buffer (terrain.wgsl's `rain_map`, shadow_rays.wgsl's).
    pub fn map(&self) -> &wgpu::Buffer {
        &self.map
    }
    /// Uploads the map's cells when they changed.
    pub fn prepare(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uploads: &mut super::upload::Uploads,
        cells: Option<&[u32]>,
    ) {
        if let Some(cells) = cells {
            uploads.buffer(encoder, &self.map, 0, bytemuck::cast_slice(cells));
        }
    }
    /// The drops, with the terrain's group 0 bound as `group`.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, group: &wgpu::BindGroup) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..4, 0..INSTANCES);
    }
}

#[cfg(test)]
mod tests {
    /// The rain's shared part is one text in the terrain's shader and the rays': the puddles the
    /// rays reflect off are the ones the terrain shows.
    #[test]
    fn the_rain_is_the_same_in_the_two_shaders() {
        let block = |source: &str| {
            let source = source.replace("\r\n", "\n");
            let from = source.find("// RAIN-BEGIN").expect("begin");
            let to = source.find("// RAIN-END").expect("end");
            source[from..to].to_string()
        };
        assert_eq!(
            block(include_str!("../terrain.wgsl")),
            block(include_str!("shadow_rays.wgsl"))
        );
    }

    /// The grids and the map the Rust side sizes are the shaders'.
    #[test]
    fn the_sizes_are_the_shaders() {
        let source = include_str!("../terrain.wgsl");
        for (name, value) in [
            ("RAIN_NEAR", super::NEAR_SIDE),
            ("RAIN_FAR", super::FAR_SIDE),
            ("RAIN_TRACKS", super::TRACKS),
            ("RAIN_FAR_TRACKS", super::FAR_TRACKS),
        ] {
            assert!(
                source.contains(&format!("const {name}: u32 = {value}u;")),
                "{name}"
            );
        }
        assert!(source.contains(&format!("const RAIN_MAP: i32 = {};", crate::rain::MAP)));
    }

    /// The terrain's shader, with the drops and the wet world, validates (naga, no GPU).
    #[test]
    fn the_terrain_shader_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("../terrain.wgsl")).unwrap();
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
    }
}
