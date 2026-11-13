//! What the game sets of RT lighting and its company (the far terrain, the fogs, the weather's
//! media, the sprite layers' materials and parallax), and what it reads back (gfx.rs may not grow).

use super::*;

impl Renderer {
    /// Whether the hardware rays hold the moving geometry (`hw_rays`): then the player's body in
    /// first person is worth building for them (`Frame::rt_body`).
    pub fn hw_rays_on(&self) -> bool {
        self.hw_rays.is_some()
    }
    /// The distant terrain's drawn tiles, for the rays' far field (`far_heights`): call after
    /// `Lod::update`; cheap unless they changed.
    pub fn set_far_terrain(&mut self, lod: &crate::lod::Lod) {
        self.far_heights.set_far_paced(
            lod.revision(),
            lod.drawn_columns(),
            std::time::Instant::now(),
        );
    }
    /// Whether the entities cast ray-traced shadows: ray-traced shadows or RT lighting (both with
    /// Linear Light) on hardware rays, which trace the entities' quads (`hw_finish`); the voxel
    /// grid alone has no entities. The game then drops vanilla's round blob under them
    /// (`EntityRenderer.extractShadow`).
    pub fn rt_entity_shadows(&self) -> bool {
        (self.rt_shadows || self.rt_lighting) && self.hw_rays.is_some()
    }
    /// Ray-traced shadows and ambient occlusion (Horizon settings, with Linear Light): the voxel
    /// grid around the camera (`voxels`), and rays toward the sun or the moon and around each
    /// surface (`shadow_rays`).
    /// `lighting`: RT lighting, the terrain lit by the rays alone (the mesher must agree:
    /// `Streaming::rt_lighting`).
    pub fn set_ray_traced(&mut self, shadows: bool, ambient: bool, lighting: bool) {
        let linear = self.tonemap.is_some();
        self.rt_shadows = shadows && linear;
        self.rt_ambient = ambient && linear;
        self.rt_lighting = lighting && linear;
        // RT lighting grades its scene: auto-exposure and bloom (tonemap.rs).
        if let Some(t) = &mut self.tonemap {
            t.set_post(self.rt_lighting);
        }
        self.voxels.enable(
            &self.device,
            self.rt_shadows || self.rt_ambient || self.rt_lighting,
        );
    }
    /// Froxel volumetrics (froxels.rs, hook 5): on or off (with RT lighting; `HORIZON_FROXELS=1`
    /// turns them on regardless), and where the camera is, which picks the medium. Call each frame
    /// with `CameraMedium { sky_light, .. }` filled; water, rain and thunder are also read from the
    /// frame.
    pub fn set_volumetrics(&mut self, on: bool, camera: froxels::CameraMedium) {
        // `HORIZON_FROXELS=1` forces them on, `=0` off (to compare).
        self.froxels.enabled = match std::env::var("HORIZON_FROXELS").as_deref() {
            Ok("0") => false,
            Ok(_) => true,
            Err(_) => on,
        };
        self.froxels.camera = camera;
    }
    /// The God Rays option (`Graphics::god_rays`): the froxels' air at `percent` of its density
    /// (0 none, 100 as measured); the water keeps its own.
    pub fn set_god_rays(&mut self, percent: u32) {
        self.froxels.strength = percent as f32 / 100.0;
    }
    /// The God Rays Quality option (`Graphics::god_rays_quality`): the froxels' grid and their
    /// per-pixel march, 0 Low to `froxels::QUALITIES - 1` Ultra (froxels.rs `quality_grid`).
    pub fn set_god_rays_quality(&mut self, quality: u32) {
        self.froxels.quality = quality.min(froxels::QUALITIES - 1);
    }
    /// The natural fogs' density, times their own (1 by default, 0 none; the live settings panel).
    pub fn set_fog_density(&mut self, density: f32) {
        self.froxels.fog_density = density;
    }
    /// The natural fogs in the froxels (natural_fog.rs): the setting, the time, the air (a real
    /// weather can set its own humidity and temperatures). Call each frame.
    pub fn set_natural_fog(&mut self, fog: natural_fog::NaturalFog) {
        self.froxels.set_fog(fog);
    }
    /// A layer of the natural fogs' relief map (`natural_fog::FogMaps::step`), for the GPU.
    pub fn queue_fog_layer(&mut self, layer: natural_fog::FogLayer) {
        self.froxels.queue_fog_layer(layer);
    }
    /// Forget the natural fogs' relief map (another world).
    pub fn clear_fog_map(&mut self) {
        self.froxels.clear_fog_map();
    }
    /// The sprite layers' ways of swaying in the wind (`wind::wind_layers`).
    pub fn set_wind_layers(&self, classes: &[u8]) {
        self.wind.set_layers(&self.queue, classes);
    }
    /// RT lighting's sun as `size` times its real size across, and the aerial perspective's air
    /// as `aerial` times as dense as real air.
    /// The water's still and flowing sprite layers, for RT lighting to draw water its own way.
    pub fn set_water_layers(&mut self, still: Option<u32>, flow: Option<u32>) {
        self.water_layers = [still, flow].map(|l| l.map_or(-1.0, |l| l as f32));
    }
    /// The lava's still and flowing sprite layers, for RT lighting to draw lava its own way
    /// (terrain.wgsl `rt_lava`): packed as `still + 4096 flow` in the water's uniform.
    pub fn set_lava_layers(&mut self, still: Option<u32>, flow: Option<u32>) {
        self.lava_layers = match (still, flow) {
            (Some(s), Some(f)) if s < 4096 && f < 4096 => (s + 4096 * f) as f32,
            (Some(s), None) if s < 4096 => (s + 4096 * s) as f32,
            _ => -1.0,
        };
    }
    /// The plain glass's sprite layers (its block's and its pane's edge), which RT lighting draws
    /// with Remix's diffuse layer over a clear dielectric where every other translucent texture
    /// is the glass's colour (terrain.wgsl `rt_glass_at`): packed as the lava's.
    pub fn set_glass_layers(&mut self, glass: Option<u32>, pane: Option<u32>) {
        self.glass_layers = match (glass, pane) {
            (Some(g), Some(p)) if g < 4096 && p < 4096 => (g + 4096 * p) as f32,
            (Some(g), None) if g < 4096 => (g + 4096 * g) as f32,
            _ => -1.0,
        };
    }
    /// The ice's and the packed ice's sprite layers, which RT lighting draws as clear and as
    /// frosted ice (terrain.wgsl `rt_ice`, mesher.rs `rt_ice_kind`): packed as the lava's, ice +
    /// 4096 packed ice.
    pub fn set_ice_layers(&mut self, ice: Option<u32>, packed: Option<u32>) {
        let on = crate::mesher::rt_ice_on();
        // Opaque packed ice (`rt_packed_ice_clear` off) keeps its layer here: it is never meshed
        // translucent then, so only the opaque terrain's shaders meet it, as compacted ice
        // (terrain.wgsl `rt_packed_ice`).
        self.ice_layers = match (ice, packed) {
            (Some(i), Some(p)) if on && i < 4096 && p < 4096 => (i + 4096 * p) as f32,
            _ => -1.0,
        };
    }
    /// RT lighting's night vision (the Night Vision setting, gfx/tonemap.wgsl `night_vision`).
    pub fn set_night_vision(&mut self, on: bool) {
        if let Some(t) = &mut self.tonemap {
            t.set_night_vision(on);
        }
    }
    /// RT lighting's shooting stars (the Shooting Stars setting, gfx/sky_stars.rs `meteors`).
    pub fn set_shooting_stars(&mut self, setting: u32) {
        self.shooting_stars = setting;
    }
    pub fn set_rt_sky(&mut self, size: u32, moon: u32, aerial: u32) {
        self.rt_sun_radius = SUN_RADIUS * size.max(1) as f32;
        self.rt_moon_radius = MOON_RADIUS * moon.max(1) as f32;
        self.rt_aerial = aerial as f32;
    }
    pub(super) fn rebuild_terrain_groups(&mut self) {
        self.terrain_groups_with(None, None, None, None, None, None, None);
    }
    /// Runs the Eye of Aldhani from `seconds` into it, or stops it (`None`); refused unless
    /// `HORIZON_METEORS=aldhani` armed it.
    /// What RT lighting's kernel leaves out, to time its parts alone in a running game
    /// (`shadow_rays::ShadowRays::set_skip`, `horizon-ctl rt-skip`).
    pub fn set_rt_skip(&mut self, skip: u32) {
        self.wind.skipped = skip & wind::SKIP_BIT != 0;
        self.shadow_rays.set_skip(&self.device, skip);
    }
    /// The screen's pixels an RT lighting ray stands for on each side, 0 by its size
    /// (`shadow_rays::Foveation::screen`, `horizon-ctl rt-scale`).
    pub fn set_rt_scale(&mut self, scale: u32) {
        shadow_rays::SCREEN_SCALE.store(scale, std::sync::atomic::Ordering::Relaxed);
    }
    /// The VR eyes' sharp zone's half angle in degrees (`shadow_rays::SHARP_SET`), 0 the default.
    pub fn set_rt_sharp(&mut self, degrees: u32) {
        shadow_rays::SHARP_SET.store(degrees, std::sync::atomic::Ordering::Relaxed);
    }
    pub fn set_aldhani(&mut self, seconds: Option<f32>) -> std::result::Result<(), &'static str> {
        let now = self.water_clock.elapsed().as_secs_f32();
        self.aldhani.set(now, seconds)
    }
    /// The seconds into the Eye of Aldhani, while it runs.
    pub fn aldhani_time(&mut self) -> Option<f32> {
        self.aldhani.time(self.water_clock.elapsed().as_secs_f32())
    }
    /// Whether the frame is drawn in linear light (Linear Light, `tonemap`).
    pub fn linear(&self) -> bool {
        self.tonemap.is_some()
    }
    /// Puts the block sprites' LabPBR maps (`RenderAssets::materials`) beside the sprite
    /// arrays, their animated maps following `animations`; `None` drops them.
    pub fn set_materials(
        &mut self,
        materials: Option<&crate::labpbr::MaterialPixels>,
        animations: &[SpriteAnimation],
        thin: &[u32],
    ) {
        self.materials = materials.map(|m| {
            materials::MaterialArrays::new(
                &self.device,
                &self.queue,
                m,
                sprite_split(),
                animations,
                thin,
                animate::SpriteAnimator::compute(&self.adapter, &self.device),
                self.adapter.get_info().backend == wgpu::Backend::Gl,
            )
        });
        if let Some(arrays) = &self.materials {
            let m = materials.expect("arrays come from maps");
            log::info!(
                "Horizon: LabPBR maps: {} normal, {} specular, {} subsurface, {:.1} MiB of video memory",
                m.maps[crate::labpbr::Map::Normal as usize].count(),
                m.maps[crate::labpbr::Map::Specular as usize].count(),
                m.maps[crate::labpbr::Map::Subsurface as usize].count(),
                arrays.bytes as f64 / f64::from(1 << 20)
            );
        }
        self.build_parallax();
    }
    /// Draws terrain with the LabPBR normal maps' relief (`gfx/parallax.rs`) while `on` and
    /// the resource packs have maps; a shader pack's material stage still wins.
    pub fn set_parallax(&mut self, on: bool) {
        if on != self.parallax_on {
            self.parallax_on = on;
            self.build_parallax();
        }
    }
    pub(super) fn build_parallax(&mut self) {
        self.parallax = None;
        let Some(materials) = self.materials.as_ref().filter(|_| self.parallax_on) else {
            return;
        };
        let shape = native_material::TerrainShape {
            globals: &self.terrain_layout,
            flat: self.flat.as_ref().map(|f| &f.draw_layout),
            sprites: self.terrain_sprites,
            format: self.world_format,
            screen: self.output.frame.format,
            linear: self.output.frame.linear_light,
            sprite_tags: &[],
            sprite_materials: &[],
            entities: &self.entity_bindings,
            weather_groups: [&self.weather.entity_layout, &self.weather.time_layout],
            pbr: None,
        };
        let normals = &materials.maps[crate::labpbr::Map::Normal as usize];
        let speculars = &materials.maps[crate::labpbr::Map::Specular as usize];
        let subsurface = &materials.maps[crate::labpbr::Map::Subsurface as usize];
        match parallax::Parallax::new(
            &self.device,
            &shape,
            [normals, speculars, subsurface],
            &materials.thin,
        ) {
            Ok(p) => self.parallax = Some(p),
            Err(e) => log::warn!("Horizon: no parallax: {e}"),
        }
    }
    /// The LabPBR maps (`Map::Normal`, `Map::Specular`, `Map::Subsurface`): their arrays and the
    /// index by sprite layer, when the resource packs have maps.
    pub fn material_arrays(&self) -> Option<&[materials::MapArrays; crate::labpbr::Map::COUNT]> {
        self.materials.as_ref().map(|m| &m.maps)
    }
}
