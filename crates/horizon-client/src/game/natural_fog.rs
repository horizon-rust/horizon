//! RT lighting's natural fogs (gfx/natural_fog.rs): each frame the air they form in (the day, the
//! hour, the rain of the last day and a half, or `HORIZON_FOG_HUMIDITY` / `HORIZON_FOG_AIR` to
//! test) and a few thousand columns of the relief map around the camera, from the loaded world
//! where it is, the far terrain elsewhere.

use super::{Dim, Game};
use horizon_client::blocks::Fluid;
use horizon_client::chunk::MIN_Y;
use horizon_client::gfx::natural_fog::{self, FogColumn, FogWeather, NaturalFog};
use horizon_world::heightmap::{HeightmapState, HeightmapType};
use horizon_world::worldgen::terrain::TerrainBlockStates;

/// Relief map columns sampled a frame: a layer (65536 columns) in under a second at 120
/// frames a second.
const COLUMNS_PER_FRAME: usize = 768;
/// How fast the wetness of a rain fades: a day and a half of game time (36000 ticks at 20 a
/// second), as frames at an assumed 60 a second.
const WETNESS_FRAMES: f32 = 36000.0 / 20.0 * 60.0;

/// A float from the environment, for testing.
fn env_f32(name: &str) -> Option<f32> {
    std::env::var(name).ok()?.trim().parse().ok()
}

impl Game<'_> {
    /// The natural fogs this frame: their state to the renderer, and the relief map's next
    /// columns (a layer finished goes to it whole).
    pub(super) fn natural_fog(&mut self, camera: glam::DVec3, sun_angle: f32) {
        if self.renderer.is_none() {
            return;
        }
        let (rain, thunder) = self.world.weather.levels(1.0);
        let wet = rain.max(thunder);
        self.fog_wetness = if wet > self.fog_wetness {
            wet
        } else {
            self.fog_wetness * (1.0 - 1.0 / WETNESS_FRAMES)
        };
        let day_time = self.world.day_time;
        let hour = natural_fog::hour(day_time);
        let mut weather = FogWeather::natural(day_time.div_euclid(24000), hour, self.fog_wetness);
        if let Some(h) = env_f32("HORIZON_FOG_HUMIDITY") {
            weather.humidity = h.clamp(0.0, 1.0);
        }
        if let Some(t) = env_f32("HORIZON_FOG_AIR") {
            weather.air_temperature = t;
        }
        // Rain itself scatters the light (the froxels' rain medium); the fogs are those of still
        // air, and of the open overworld: not under water, nor deep in a cave.
        let at = camera.floor().as_ivec3();
        let sky = f32::from(self.world.light(at.x, at.y, at.z) >> 4);
        let open = if self.dim != Dim::Overworld
            || self.camera_fluid(camera.to_array()) != horizon_client::camera::CameraFluid::None
        {
            0.0
        } else {
            ((sky - 6.0) / 6.0).clamp(0.0, 1.0) * (1.0 - wet)
        };
        let sun = horizon_client::celestial::direction(
            sun_angle,
            self.settings.horizon.sun_path_tilt as f32,
        );
        let fog = NaturalFog {
            setting: self.settings.horizon.natural_fog,
            day_time,
            weather,
            sun_height: sun[1],
            wind: [0.8, 0.3],
            open,
        };
        let layer = if fog.active() && self.settings.horizon.rt_lighting {
            let mut maps = std::mem::take(&mut self.fog_maps);
            let layer = maps.step(camera, COLUMNS_PER_FRAME, |x, z| self.fog_column(x, z));
            self.fog_maps = maps;
            layer
        } else {
            None
        };
        if let Some(renderer) = self.renderer.as_mut() {
            renderer.set_natural_fog(fog);
            if let Some(layer) = layer {
                renderer.queue_fog_layer(layer);
            }
        }
    }

    /// The relief map's column at block x, z: the loaded world's (its ground under water and
    /// leaves), else the far terrain's.
    fn fog_column(&self, x: i32, z: i32) -> Option<FogColumn> {
        let biomes = self.res.biomes();
        let climate = |biome: u16| {
            let b = biomes.get(biome);
            natural_fog::biome_climate(&b.name, b.downfall, b.temperature)
        };
        let column = |ground: i32, water: Option<i32>, biome: u16, heated: bool| {
            let c = climate(biome);
            FogColumn {
                ground: ground as f32,
                surface: water.unwrap_or(ground).max(ground) as f32,
                water: water.is_some_and(|w| w > ground),
                humid: c.humid,
                calm: c.calm,
                cold: c.air,
                warmth: c.water
                    + if heated {
                        natural_fog::HEATED_WATER
                    } else {
                        0.0
                    },
            }
        };
        if self.world.loaded(x, z) {
            let properties = &self.pack.assets.properties;
            let facts = self.res.facts();
            let top = self.world.motion_blocking(x, z, |s| {
                HeightmapType::MotionBlocking.is_opaque(properties.heightmap_state(s))
            });
            if top <= MIN_Y {
                return None;
            }
            // Down through water, leaves and what lets one walk through, to the ground (24 blocks
            // at most: deeper water needs no ground).
            let mut y = top - 1;
            let mut water = None;
            let mut heated = false;
            for _ in 0..24 {
                let state = self.world.block(x, y, z);
                if facts.get(state).fluid == Fluid::Water {
                    water.get_or_insert(y + 1);
                } else if properties
                    .heightmap_state(state)
                    .blocks_motion_in_heightmap_no_leaves()
                {
                    // Water on magma (its bubbles) is warm: a hot spring.
                    heated = water.is_some() && &*facts.get(state).name == "minecraft:magma_block";
                    break;
                }
                y -= 1;
            }
            let biome = self
                .world
                .chunks
                .get(&(x >> 4, z >> 4))
                .map_or(0, |c| c.biome(x, y + 1, z));
            return Some(column(y + 1, water, biome, heated));
        }
        let surface = self.lod.as_ref()?.surface(x, z)?;
        let water = surface
            .water
            .filter(|&(_, state)| self.res.facts().get(state).fluid == Fluid::Water)
            .map(|(h, _)| h);
        Some(column(surface.height, water, surface.biome, false))
    }
}
