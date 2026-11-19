//! A mod density function type named in noise settings, as every reader of the density graph sees
//! it: chunk fill, far terrain and structure base height find the same surface, the climate
//! samplers read it, and volume readers call the mod once per volume. The only tests of this
//! binary that install the process-wide table; their type ids are their own.
use super::super::mod_density_functions::{
    ModDensityFunction, ModDensityFunctionType, ModDensityFunctionTypes,
    install_mod_density_function_types,
};
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// `0.05 × (top − y)`, clamped to ±100, where `top = base + x/8 − z/16 + seed mod 7`: linear in
/// x, y and z (so far terrain's lattice interpolation is exact) and never zero at an integer y.
#[derive(Clone, Copy)]
struct Ramp {
    base: f32,
}
impl Ramp {
    fn top(&self, x: i32, z: i32, seed: i64) -> f32 {
        self.base + x as f32 / 8.0 - z as f32 / 16.0 + seed.rem_euclid(7) as f32
    }
    fn value(&self, [x, y, z]: [i32; 3], seed: i64) -> f32 {
        (0.05 * (self.top(x, z, seed) - y as f32)).clamp(-100.0, 100.0)
    }
    /// The exclusive top face of the solid column.
    fn surface(&self, x: i32, z: i32, seed: i64) -> i32 {
        self.top(x, z, seed).floor() as i32 + 1
    }
}

/// A type whose functions are [`Ramp`]s, counting how they are asked.
struct RampType {
    points: &'static AtomicUsize,
    volumes: &'static AtomicUsize,
}
struct RampFunction {
    ramp: Ramp,
    points: &'static AtomicUsize,
    volumes: &'static AtomicUsize,
}
impl ModDensityFunctionType for RampType {
    fn decode(&self, json: &str) -> Result<Arc<dyn ModDensityFunction>, String> {
        let value: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        let base = value["base"].as_f64().ok_or("a ramp needs its base")?;
        Ok(Arc::new(RampFunction {
            ramp: Ramp { base: base as f32 },
            points: self.points,
            volumes: self.volumes,
        }))
    }
}
impl ModDensityFunction for RampFunction {
    fn range(&self) -> [f32; 2] {
        [-100.0, 100.0]
    }
    fn varies_with_y(&self) -> bool {
        true
    }
    fn sample(&self, position: [i32; 3], seed: i64) -> Result<f32, String> {
        self.points.fetch_add(1, Ordering::Relaxed);
        Ok(self.ramp.value(position, seed))
    }
    fn fill(
        &self,
        origin: [i32; 3],
        step: [i32; 3],
        count: [usize; 3],
        seed: i64,
        out: &mut [f32],
    ) -> Result<(), String> {
        self.volumes.fetch_add(1, Ordering::Relaxed);
        assert_eq!(out.len(), count.iter().product::<usize>());
        let mut index = 0;
        for z in 0..count[2] as i32 {
            for x in 0..count[0] as i32 {
                for y in 0..count[1] as i32 {
                    let at = [x, y, z];
                    let position = std::array::from_fn(|a| origin[a] + at[a] * step[a]);
                    out[index] = self.ramp.value(position, seed);
                    index += 1;
                }
            }
        }
        Ok(())
    }
}

/// `{"value": v}` everywhere, never depending on y.
struct Flat;
impl ModDensityFunctionType for Flat {
    fn decode(&self, json: &str) -> Result<Arc<dyn ModDensityFunction>, String> {
        let value: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
        Ok(Arc::new(FlatValue(
            value["value"].as_f64().ok_or("no value")? as f32,
        )))
    }
}
struct FlatValue(f32);
impl ModDensityFunction for FlatValue {
    fn range(&self) -> [f32; 2] {
        [self.0, self.0]
    }
    fn varies_with_y(&self) -> bool {
        false
    }
    fn sample(&self, _: [i32; 3], _: i64) -> Result<f32, String> {
        Ok(self.0)
    }
    fn fill(
        &self,
        _: [i32; 3],
        _: [i32; 3],
        _: [usize; 3],
        _: i64,
        out: &mut [f32],
    ) -> Result<(), String> {
        out.fill(self.0);
        Ok(())
    }
}

/// A function whose code fails (as a contained panic comes back from the adapter).
struct Fails;
impl ModDensityFunctionType for Fails {
    fn decode(&self, _: &str) -> Result<Arc<dyn ModDensityFunction>, String> {
        Ok(Arc::new(Fails))
    }
}
impl ModDensityFunction for Fails {
    fn range(&self) -> [f32; 2] {
        [-1.0, 1.0]
    }
    fn varies_with_y(&self) -> bool {
        true
    }
    fn sample(&self, _: [i32; 3], _: i64) -> Result<f32, String> {
        Err("the mod's sample panicked".into())
    }
    fn fill(
        &self,
        _: [i32; 3],
        _: [i32; 3],
        _: [usize; 3],
        _: i64,
        _: &mut [f32],
    ) -> Result<(), String> {
        Err("the mod's fill panicked".into())
    }
}

static RAMP_POINTS: AtomicUsize = AtomicUsize::new(0);
static RAMP_VOLUMES: AtomicUsize = AtomicUsize::new(0);
static COUNTED_POINTS: AtomicUsize = AtomicUsize::new(0);
static COUNTED_VOLUMES: AtomicUsize = AtomicUsize::new(0);
static DECORATION_POINTS: AtomicUsize = AtomicUsize::new(0);
static DECORATION_VOLUMES: AtomicUsize = AtomicUsize::new(0);

/// Every test's types, installed by whichever test runs first.
fn install() {
    let _ = install_mod_density_function_types(ModDensityFunctionTypes::from([
        (
            "worldgen_test:ramp".to_owned(),
            Arc::new(RampType {
                points: &RAMP_POINTS,
                volumes: &RAMP_VOLUMES,
            }) as _,
        ),
        (
            "worldgen_test:counted".to_owned(),
            Arc::new(RampType {
                points: &COUNTED_POINTS,
                volumes: &COUNTED_VOLUMES,
            }) as _,
        ),
        (
            "worldgen_test:decoration".to_owned(),
            Arc::new(RampType {
                points: &DECORATION_POINTS,
                volumes: &DECORATION_VOLUMES,
            }) as _,
        ),
        ("worldgen_test:flat".to_owned(), Arc::new(Flat) as _),
        ("worldgen_test:fails".to_owned(), Arc::new(Fails) as _),
    ]));
}

fn resources() -> Option<std::path::PathBuf> {
    let root = std::env::var_os("HORIZON_TEST_RESOURCES")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3")
        });
    if root.join("worldgen/world_presets.json").exists() {
        Some(root)
    } else {
        eprintln!("skipping: no worldgen bundle at {}", root.display());
        None
    }
}

fn data(root: &Path) -> Arc<GeneratorData> {
    let registry = Box::leak(Box::new(
        crate::level::data::load_registry(root).expect("registry"),
    ));
    GeneratorData::load(&root.join("worldgen"), registry).expect("worldgen data")
}

/// The overworld's noise settings with `edit` applied, over the overworld preset's generator, or
/// over a single carver-free void biome when `void`.
fn pipeline(
    data: &Arc<GeneratorData>,
    seed: i64,
    void: bool,
    edit: impl FnOnce(&mut Value),
) -> Result<Arc<Pipeline<XoroshiroRandom>>, GeneratorError> {
    pipeline_with(data, seed, void, false, edit)
}

/// `pipeline`, with the preset's structures when `structures`.
fn pipeline_with(
    data: &Arc<GeneratorData>,
    seed: i64,
    void: bool,
    structures: bool,
    edit: impl FnOnce(&mut Value),
) -> Result<Arc<Pipeline<XoroshiroRandom>>, GeneratorError> {
    let preset: Value = serde_json::from_str(&data.world_presets["minecraft:normal"]).unwrap();
    let mut generator = preset["dimensions"]["minecraft:overworld"]["generator"].clone();
    let mut settings: Value =
        serde_json::from_str(&data.settings[generator["settings"].as_str().unwrap()]).unwrap();
    edit(&mut settings);
    if void {
        generator["biome_source"] =
            serde_json::json!({"type": "minecraft:fixed", "biome": "minecraft:the_void"});
    }
    generator["settings"] = settings.clone();
    Ok(Arc::new(Pipeline::new(
        data.clone(),
        seed,
        XoroshiroRandom::new(seed),
        &generator,
        &settings,
        structures,
        Some((-64, 384)),
        None,
    )?))
}

/// Terrain from the mod's ramp alone: no aquifers or veins, one carver-free biome, no structures.
fn ramp_world(
    data: &Arc<GeneratorData>,
    seed: i64,
    kind: &str,
) -> (Arc<Pipeline<XoroshiroRandom>>, GeneratorWorker) {
    let pipeline = pipeline(data, seed, true, |settings| {
        settings["aquifers_enabled"] = false.into();
        settings["ore_veins_enabled"] = false.into();
        settings["noise_router"]["final_density"] =
            serde_json::json!({"type": kind, "base": 70.37});
    })
    .expect("pipeline");
    let worker = OverworldGenerator {
        pipeline: pipeline.clone(),
        noise_clamped: None,
    }
    .worker();
    (pipeline, worker)
}

/// The top face of chunk fill's solid column at `x, z`.
fn filled_top(chunk: &ProtoChunk, x: i32, z: i32) -> i32 {
    (-64..320)
        .rev()
        .find(|&y| chunk.block_name(x, y, z) != "minecraft:air")
        .map_or(-64, |y| y + 1)
}

/// For two seeds, chunk fill, far terrain's sampler and structures' base height all put the
/// surface where the mod's function crosses zero.
#[test]
fn chunk_fill_far_terrain_and_structure_height_read_the_mods_density() {
    install();
    let Some(root) = resources() else { return };
    let data = data(&root);
    let ramp = Ramp { base: 70.37 };
    for seed in [12345, -7] {
        let (pipeline, mut worker) = ramp_world(&data, seed, "worldgen_test:ramp");
        for (cx, cz) in [(0, 0), (-2, 1)] {
            let chunk = worker.generate_proto(ChunkPos::new(cx, cz)).unwrap();
            let (x0, z0) = (cx * 16, cz * 16);
            let far = worker.lod_grid(x0, z0, 4, 4, false).unwrap();
            for (i, (sample, _)) in far.iter().enumerate() {
                let (x, z) = (x0 + (i % 4) as i32 * 4, z0 + (i / 4) as i32 * 4);
                let expected = ramp.surface(x, z, seed);
                assert_eq!(filled_top(&chunk, x, z), expected, "chunk fill at {x} {z}");
                assert_eq!(sample.unwrap().height, expected, "far terrain at {x} {z}");
                assert_eq!(
                    pipeline.base_height(x, z, HeightmapType::WorldSurfaceWg),
                    expected,
                    "structure base height at {x} {z}"
                );
            }
        }
    }
}

/// Chunk fill, far terrain and structure height pass whole volumes to the mod, never single
/// points, and the node keeps the graph pointwise, so chunk fill can reuse lattice samples.
#[test]
fn volume_readers_call_fill_not_sample() {
    install();
    let Some(root) = resources() else { return };
    let data = data(&root);
    let (pipeline, mut worker) = ramp_world(&data, 3, "worldgen_test:counted");
    assert!(horizon_core::density::is_pointwise(
        pipeline.router.get(RouterField::FinalDensity)
    ));
    worker.generate_proto(ChunkPos::new(1, -1)).unwrap();
    worker.lod_grid(0, 0, 8, 8, false).unwrap();
    worker.lod_grid(0, 0, 32, 8, true).unwrap();
    pipeline.base_height(5, 9, HeightmapType::WorldSurfaceWg);
    assert_eq!(COUNTED_POINTS.load(Ordering::Relaxed), 0);
    assert!(COUNTED_VOLUMES.load(Ordering::Relaxed) > 0);
}

/// A Y-free mod type as the router's `continents` moves every climate reader: the biomes chunks
/// and `noise_biomes` pick, and `/locate biome`. Continentalness −1.15 is the mushroom fields',
/// everywhere above the caves (y 240 here).
#[test]
fn the_climate_samplers_read_a_mod_continents() {
    install();
    let Some(root) = resources() else { return };
    let data = data(&root);
    let pipeline = pipeline(&data, 12345, false, |settings| {
        settings["noise_router"]["continents"] =
            serde_json::json!({"type": "worldgen_test:flat", "value": -1.15});
    })
    .expect("pipeline");
    let mut worker = OverworldGenerator {
        pipeline,
        noise_clamped: None,
    }
    .worker();
    let quarts: Vec<[i32; 3]> = (-8..8).map(|i| [i * 97, 60, i * -53]).collect();
    for biome in worker.noise_biomes(&quarts) {
        assert_eq!(biome.as_deref(), Some("minecraft:mushroom_fields"));
    }
    let chunk = worker.generate_proto(ChunkPos::new(40, -9)).unwrap();
    assert_eq!(
        chunk.biome_at(40 * 16, 240, -9 * 16),
        "minecraft:mushroom_fields"
    );
    let found = worker.find_closest_biome(&BiomeSearch {
        allowed: &|biome| biome == "minecraft:mushroom_fields",
        origin: [1000, 240, -1000],
        radius: 64,
        horizontal: 8,
        vertical: 16,
        min_y: 200,
        max_y: 319,
    });
    assert_eq!(
        found.map(|(biome, _)| biome).as_deref(),
        Some("minecraft:mushroom_fields")
    );
}

/// Noise settings naming a type nobody registered, or one whose decoder refuses the object, fail
/// to load with an error naming the type; nothing panics.
#[test]
fn an_unknown_or_refused_mod_type_is_a_load_error_naming_it() {
    install();
    let Some(root) = resources() else { return };
    let data = data(&root);
    let error = |final_density: Value| {
        pipeline(&data, 0, true, |settings| {
            settings["noise_router"]["final_density"] = final_density;
        })
        .err()
        .expect("refused")
        .to_string()
    };
    let unknown = error(serde_json::json!({"type": "worldgen_test:gone"}));
    assert!(
        unknown.contains("unknown density type: worldgen_test:gone"),
        "{unknown}"
    );
    let refused = error(serde_json::json!({"type": "worldgen_test:ramp"}));
    assert!(
        refused.contains("density type worldgen_test:ramp: a ramp needs its base"),
        "{refused}"
    );
}

/// A mod function whose code fails mid-generation fails that chunk's generation with an error
/// naming the type and its mod, on the worker that asked, instead of unwinding through it. From
/// then on the world answers nothing from samples: no far terrain, no spawn, and no structure
/// height or start is kept.
#[test]
fn a_failing_mod_function_fails_the_world_naming_it() {
    install();
    let Some(root) = resources() else { return };
    let data = data(&root);
    let pipeline = pipeline_with(&data, 1, false, true, |settings| {
        settings["noise_router"]["final_density"] =
            serde_json::json!({"type": "worldgen_test:fails"});
    })
    .expect("pipeline");
    let generator = OverworldGenerator {
        pipeline: pipeline.clone(),
        noise_clamped: None,
    };
    let mut worker = generator.worker();
    let named = "density function type worldgen_test:fails of mod worldgen_test failed: the mod's";
    let error = worker
        .generate_proto(ChunkPos::new(0, 0))
        .err()
        .expect("generation fails")
        .to_string();
    assert!(error.contains(named), "{error}");
    assert_eq!(
        worker.mod_density_failure(),
        generator.mod_density_failure()
    );
    assert!(worker.state.heights.is_empty());
    let cached = pipeline.structures.as_ref().unwrap().start_census();
    for at in [
        ChunkPos::new(5, 5),
        ChunkPos::new(-9, 4),
        ChunkPos::new(30, -30),
    ] {
        assert!(worker.generate_proto(at).is_err());
        assert!(worker.structure_starts(at).is_empty());
    }
    assert!(worker.state.heights.is_empty());
    assert_eq!(pipeline.structures.as_ref().unwrap().start_census(), cached);
    let far = worker
        .lod_grid(0, 0, 8, 4, false)
        .expect_err("far terrain fails");
    assert!(far.to_string().contains(named), "{far}");
    let spawn = generator.spawn_position().expect_err("spawn search fails");
    assert!(spawn.to_string().contains(named), "{spawn}");
}

/// A failing mod `continents` reaches every climate reader: each gives an error or no answer,
/// and `/locate biome` stops at the first failure.
#[test]
fn every_climate_reader_of_a_failing_mod_function_returns() {
    install();
    let Some(root) = resources() else { return };
    let data = data(&root);
    let pipeline = pipeline(&data, 12345, false, |settings| {
        settings["noise_router"]["continents"] = serde_json::json!({"type": "worldgen_test:fails"});
    })
    .expect("pipeline");
    let generator = OverworldGenerator {
        pipeline,
        noise_clamped: None,
    };
    let mut worker = generator.worker();
    let found = worker.find_closest_biome(&BiomeSearch {
        allowed: &|_| true,
        origin: [0, 64, 0],
        radius: 6400,
        horizontal: 8,
        vertical: 16,
        min_y: -64,
        max_y: 319,
    });
    assert_eq!(found, None);
    assert!((worker.mod_density_failure()).is_some_and(|f| f.contains("worldgen_test:fails")));
    assert!(
        worker
            .noise_biomes(&[[0, 60, 0], [97, 60, -53]])
            .iter()
            .all(Option::is_none)
    );
    assert!(worker.generate_proto(ChunkPos::new(3, -2)).is_err());
    assert!(generator.spawn_position().is_err());
}

/// Decoration reads no density: with a counted mod node in the terrain and climate densities of
/// the vanilla overworld, decorating a region that holds structure pieces calls the mod not once.
/// (So a mod's failure can't happen part way through a decoration: the chunks' generation, which
/// does call it, is where it fails.)
#[test]
fn decoration_never_calls_a_mod_density_function() {
    install();
    let Some(root) = resources() else { return };
    let data = data(&root);
    let pipeline = pipeline_with(&data, 12345, false, true, |settings| {
        let router = &mut settings["noise_router"];
        for field in ["final_density", "continents", "erosion", "depth"] {
            let vanilla = router[field].take();
            router[field] = serde_json::json!({
                "type": "minecraft:add",
                "left": vanilla,
                "right": {
                    "type": "minecraft:mul",
                    "left": 1e-6,
                    "right": {"type": "worldgen_test:decoration", "base": 70.37},
                },
            });
        }
    })
    .expect("pipeline");
    let mut worker = OverworldGenerator {
        pipeline,
        noise_clamped: None,
    }
    .worker();
    let center = (0..24)
        .flat_map(|x| (0..24).map(move |z| ChunkPos::new(x, z)))
        .find(|&at| {
            let chunk = worker.generate_proto(at).unwrap();
            !chunk.structure_starts.is_empty()
        })
        .expect("a chunk that structure pieces reach");
    let mut region: [ProtoChunk; 9] = std::array::from_fn(|i| {
        let at = ChunkPos::new(center.x + i as i32 % 3 - 1, center.z + i as i32 / 3 - 1);
        worker.generate_proto(at).unwrap()
    });
    assert!(DECORATION_VOLUMES.load(Ordering::Relaxed) > 0);
    let before = (
        DECORATION_POINTS.load(Ordering::Relaxed),
        DECORATION_VOLUMES.load(Ordering::Relaxed),
    );
    worker.decorate(&mut region).unwrap();
    assert!(region[4].is_decorated());
    let after = (
        DECORATION_POINTS.load(Ordering::Relaxed),
        DECORATION_VOLUMES.load(Ordering::Relaxed),
    );
    assert_eq!(
        after, before,
        "decorating {center:?} called the mod's density function"
    );
}
