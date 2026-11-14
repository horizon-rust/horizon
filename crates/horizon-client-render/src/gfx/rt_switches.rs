//! RT lighting's switches (its `HORIZON_RT_*` and prepass variables), its small helpers and the
//! prepass's own types (gfx.rs may not grow).

use super::*;

/// `Renderer::skins_entities`' `HORIZON_RT_GPU_MOBS` (0: off).
pub(super) fn rt_gpu_mobs() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("HORIZON_RT_GPU_MOBS").as_deref() != Ok("0"))
}
/// Whether RT lighting's depth prepass draws its opaque quads without the alpha test
/// (`Renderer::split_prepass`): on unless `HORIZON_RT_PREPASS_SPLIT=0`.
pub(super) fn prepass_split() -> bool {
    static SPLIT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SPLIT.get_or_init(|| std::env::var("HORIZON_RT_PREPASS_SPLIT").map_or(true, |v| v != "0"))
}
/// Whether VR's depth prepass for the rays is drawn at their resolution, a depth pixel for each
/// mask texel of 2x2 pixels (`Renderer::rt_prepass`), rather than at the eye's: a quarter of the
/// pixels, which its cost follows (0.96 to 1.42 ms from 3132x3016 to 4400x3520 an eye). The rays
/// read one depth pixel a texel either way (shadow_rays.wgsl `pixel_of`), now the surface at the
/// texel's centre, and its normal from the neighbours two pixels away; the terrain's screen-space
/// reflections and water depth read the prepass at that resolution. On unless
/// `HORIZON_RT_PREPASS_HALF=0`.
pub(super) fn half_prepass() -> bool {
    static HALF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *HALF.get_or_init(|| std::env::var("HORIZON_RT_PREPASS_HALF").map_or(true, |v| v != "0"))
}
/// How the world's solid terrain starts its depth (`Renderer::render`), to time what shading the
/// hidden faces costs; off by default until measured. `HORIZON_WORLD_PREPASS=1`: from the rays'
/// prepass (only with the TAA's jitter off); `2`: from a depth pass of its own before it, with the
/// world's own (jittered) matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WorldPrepass {
    Off,
    Rays,
    Own,
}
pub(super) fn world_prepass() -> WorldPrepass {
    static MODE: std::sync::OnceLock<WorldPrepass> = std::sync::OnceLock::new();
    *MODE.get_or_init(
        || match std::env::var("HORIZON_WORLD_PREPASS").ok().as_deref() {
            Some("1") => WorldPrepass::Rays,
            Some("2") => WorldPrepass::Own,
            _ => WorldPrepass::Off,
        },
    )
}
/// `HORIZON_SKY_LAST=1`: RT lighting's sky drawn after the solid terrain (`SkyLast`); off by
/// default until measured.
pub(super) fn sky_last() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("HORIZON_SKY_LAST").is_ok_and(|v| v == "1"))
}
/// RT lighting's sky, stars, planets and celestial bodies depth-tested at the far plane (depth 0,
/// which only pixels no solid terrain covers still hold), so that drawn after the solid terrain
/// the sky's fragment shader (the atmosphere, the Milky Way, the meteors) runs only where the
/// sky shows, not under the terrain that covers most of the screen and draws over it anyway.
/// The picture is the same: the sky writes no depth, and the solid terrain is opaque.
pub(super) struct SkyLast {
    pub(super) sky: wgpu::RenderPipeline,
    pub(super) stars: wgpu::RenderPipeline,
    pub(super) planets: wgpu::RenderPipeline,
    /// The celestial pass (`vs_sky_entity`: its quads put at the far plane).
    pub(super) celestial: wgpu::RenderPipeline,
}
/// A shadow ray's budget: 1 an empty brick crossed, 4 a full one; past it the light is taken
/// to reach.
pub(super) const RT_STEPS: f32 = 48.0;
/// How much the ray-traced ambient occlusion closes a surface's light around it at most.
pub(super) const RT_AMBIENT: f32 = 0.6;
/// `HORIZON_RT_DEBUG` (shadow_rays.rs, tonemap.rs, terrain.wgsl's RT lighting at 7).
pub(super) fn rt_debug() -> u32 {
    static DEBUG: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *DEBUG.get_or_init(|| {
        std::env::var("HORIZON_RT_DEBUG")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0)
    })
}
/// `HORIZON_RT_STEPS`: another budget, for measuring.
pub(super) fn rt_steps() -> f32 {
    static STEPS: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *STEPS.get_or_init(|| {
        std::env::var("HORIZON_RT_STEPS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(RT_STEPS)
    })
}
/// The translucent terrain's cover over the rays' prepass (`Renderer::draw_cover`): 1 where water
/// or glass lies in front of the solid terrain.
pub(super) const COVER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
/// Whether the hit block's outline sways with its block (`Frame::outline_wind`):
/// `HORIZON_OUTLINE_WIND=0` keeps it still, as before.
pub(super) fn outline_sways() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("HORIZON_OUTLINE_WIND").map_or(true, |v| v.trim() != "0"))
}
/// The share of the sun's or the moon's direct light at elevation sine `y`: none at the horizon,
/// all of it from `HORIZON_SUN_RISE` degrees up (3 by default; 0: all of it at once, as before).
pub(super) fn sun_rise(y: f32) -> f32 {
    static FULL: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    let full = *FULL.get_or_init(|| {
        std::env::var("HORIZON_SUN_RISE")
            .ok()
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite())
            .map_or(3.0, |v| v.clamp(0.0, 20.0))
            .to_radians()
            .sin()
    });
    if full <= 0.0 {
        return 1.0;
    }
    let t = (y / full).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
/// RT lighting's ambient light by default (`HORIZON_RT_AMBIENT`, else 0.003): the RT Ambient
/// Light option's default (`Graphics::rt_ambient_light`).
pub fn rt_ambient_default() -> f64 {
    shadow_rays::ambient()
}
/// `HORIZON_RT_SHORE=1`: RT lighting's surf on a beach's shallows and the wet sand at the water's
/// edge (terrain.wgsl `rt_shore`, `globals.earth.w`), off by default: the surf's veil hid the bed
/// and its caustics over a whole terrace of sand.
pub(super) fn rt_shore(ambient: f32) -> f32 {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    // And RT lighting's ambient light as its fraction (entity.wgsl `rt_entity_light`: an entity
    // is not traced, the ambient is added there), under a half.
    f32::from(u8::from(*ON.get_or_init(|| {
        std::env::var("HORIZON_RT_SHORE").is_ok_and(|v| v == "1")
    }))) + ambient.clamp(0.0, 0.49)
}
/// `HORIZON_RT_HAND_SHADOW=1`: the view model stays in the hardware rays beside the player's own
/// body (`Renderer::hw_finish`), as before.
pub(super) fn rt_hand_shadow() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("HORIZON_RT_HAND_SHADOW").is_ok_and(|v| v == "1"))
}
/// Whether the rays look for water or glass in front of a texel only where the translucent
/// terrain's cover says some lies (shadow_rays_hw.wgsl `hw_reflect`): on unless
/// `HORIZON_RT_PSR_COVER=0`, which traces that ray from every texel as before.
pub(super) fn psr_cover() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("HORIZON_RT_PSR_COVER").map_or(true, |v| v != "0"))
}
/// A target for the translucent terrain's cover of `size`, with `layers` layers.
pub(super) fn cover_texture(device: &wgpu::Device, size: [u32; 2], layers: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Translucent terrain's cover for the rays"),
        size: wgpu::Extent3d {
            width: size[0].max(1),
            height: size[1].max(1),
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: COVER_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}
/// The first eye's draw lists, for the second (`Renderer::draw_frame`): phase 1's end, the
/// solid draws' end, the translucent draws, whether occlusion culling tested them, and the
/// opaque and cutout split with GPU culling's solid sections (`opaque_end`, `cutout_start`,
/// `phase2_cutout`, `gpu_solid`).
pub(super) type EyeLists = (u32, u32, u32, bool, (usize, usize, u32, usize));
/// VR's depth prepass for both eyes at once (multiview): the first eye draws the solid terrain
/// into a two-layer depth, each layer from its eye, and the second eye's rays take layer 1.
pub(super) struct EyeDepths {
    pub(super) pipeline: wgpu::RenderPipeline,
    /// The same without the alpha test, for the opaque quads (`prepass_split`).
    pub(super) opaque: wgpu::RenderPipeline,
    /// Both eyes' matrices (terrain_views.wgsl) and their group.
    pub(super) matrices: wgpu::Buffer,
    pub(super) group: wgpu::BindGroup,
    /// The two-layer depth for the eyes' size: its view to draw into and each layer's.
    pub(super) layers: Option<([u32; 2], wgpu::TextureView, [wgpu::TextureView; 2])>,
    /// The second eye's layer, drawn by the first eye this frame.
    pub(super) next: Option<wgpu::TextureView>,
    /// The translucent terrain's cover over both layers (`Renderer::draw_cover`): its pipeline,
    /// its two-layer target as `layers`, and the second eye's layer this frame.
    pub(super) cover: wgpu::RenderPipeline,
    pub(super) covers: Option<([u32; 2], wgpu::TextureView, [wgpu::TextureView; 2])>,
    pub(super) next_cover: Option<wgpu::TextureView>,
}
