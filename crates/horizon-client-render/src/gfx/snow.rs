//! RT lighting's physical snow (`crate::snow`): the flakes' draw. The cover is shading in the
//! terrain's shader (terrain.wgsl `rt_snow`), the columns where it snows the rain map's
//! (`gfx/rain.rs`, bit 11 of a column).
//!
//! The flakes are instanced quads with no vertex data (terrain.wgsl `vs_snow`), as the rain's
//! drops, but laid out for snow: a flake falls about a metre a second, so the wind carries it
//! further sideways than down, out of the cell it started in. The cells (`NEAR_SIDE`² one-block
//! ones around the camera, `FAR_SIDE`² four-block ones out to 64 blocks) tile the world, each tile
//! the same, and a flake is drawn at the copy of its place nearest the camera: the wind blows the
//! whole field through the grid with no edge for it to empty from. Both eyes of a headset see the
//! same flakes (their sizes, speeds and spirals hashed from the cells' world coordinates). Drawn
//! with the rain's drops, over the resolved scene, blended (premultiplied), depth tested against
//! the world's and not written.

/// The grids' sides in cells, and the tracks a cell (terrain.wgsl's `SNOW_*` constants).
pub const NEAR_SIDE: u32 = 48;
pub const FAR_SIDE: u32 = 32;
pub const TRACKS: u32 = 64;
pub const FAR_TRACKS: u32 = 32;
/// Instances a frame: every track of both grids.
pub const INSTANCES: u32 = NEAR_SIDE * NEAR_SIDE * TRACKS + FAR_SIDE * FAR_SIDE * FAR_TRACKS;

pub(super) struct Snow {
    pipeline: wgpu::RenderPipeline,
}
impl Snow {
    /// `layout` and `shader` are the terrain's (group 0 holds the rain map, binding 23, and the
    /// wind, binding 25).
    pub fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
    ) -> Self {
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Snow flakes"),
            layout: Some(layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_snow"),
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
                entry_point: Some("fs_snow"),
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
        Self { pipeline }
    }
    /// The flakes, with the terrain's group 0 bound as `group`.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, group: &wgpu::BindGroup) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, group, &[]);
        pass.draw(0..4, 0..INSTANCES);
    }
}

#[cfg(test)]
mod tests {
    /// The grids the Rust side sizes are the shader's.
    #[test]
    fn the_sizes_are_the_shaders() {
        let source = include_str!("../terrain.wgsl");
        for (name, value) in [
            ("SNOW_NEAR", super::NEAR_SIDE),
            ("SNOW_FAR", super::FAR_SIDE),
            ("SNOW_TRACKS", super::TRACKS),
            ("SNOW_FAR_TRACKS", super::FAR_TRACKS),
        ] {
            assert!(
                source.contains(&format!("const {name}: u32 = {value}u;")),
                "{name}"
            );
        }
    }
}
