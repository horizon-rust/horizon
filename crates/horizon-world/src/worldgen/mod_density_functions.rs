//! Density function types a mod registers, handed over once at startup and looked up when density
//! JSON names one (`{"type": "modid:plates", ...}`). A decoded one is a node of the density graph,
//! so every reader of the graph sees it. Like a mod's biome source it runs on the worldgen worker
//! threads, so it keeps no mutable state and has no `Host`.
use horizon_core::{
    density::{DensitySampler, SampleError, SamplerContext, graph::SeededDensity},
    noise::{DensityError, DensityVolume},
};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

/// A density function type's engine side, as the executable's adapter sees it.
pub trait ModDensityFunctionType: Send + Sync {
    /// One `{"type": "modid:name", ...}` object, as JSON text, into its function; `Err` refuses
    /// the density JSON that holds it.
    fn decode(&self, json: &str) -> Result<Arc<dyn ModDensityFunction>, String>;
}

/// A decoded node: a pure function of the world seed and the block position. A panic in it is not
/// caught: it unwinds out of whatever samples, to the worker's panic hook (the crash path).
pub trait ModDensityFunction: Send + Sync {
    /// `[min, max]` of every value.
    fn range(&self) -> [f32; 2];
    /// `false`: the value never depends on y.
    fn varies_with_y(&self) -> bool;
    fn sample(&self, position: [i32; 3], seed: i64) -> f32;
    /// `count` positions from `origin`, `step` apart, into `out` (exactly that many values, y
    /// fastest, then x, then z).
    fn fill(&self, origin: [i32; 3], step: [i32; 3], count: [usize; 3], seed: i64, out: &mut [f32]);
}

/// The installed density function types, `modid:plates` → type. Set once, before any world opens
/// (the executable hands the mods' registrations over).
pub type ModDensityFunctionTypes = BTreeMap<String, Arc<dyn ModDensityFunctionType>>;

static MOD_DENSITY_FUNCTION_TYPES: OnceLock<ModDensityFunctionTypes> = OnceLock::new();
static EMPTY: ModDensityFunctionTypes = BTreeMap::new();

/// Install the process-wide table once; a second installation keeps the first.
pub fn install_mod_density_function_types(
    types: ModDensityFunctionTypes,
) -> Result<(), ModDensityFunctionTypes> {
    MOD_DENSITY_FUNCTION_TYPES.set(types)
}

/// The installed density function types: empty until the executable installs the mods'.
pub fn mod_density_function_types() -> &'static ModDensityFunctionTypes {
    MOD_DENSITY_FUNCTION_TYPES.get().unwrap_or(&EMPTY)
}

/// A decoded mod node in the graph, waiting for the compiler's seed.
pub(super) struct ModDensity {
    pub(super) id: String,
    pub(super) function: Arc<dyn ModDensityFunction>,
}
impl SeededDensity for ModDensity {
    fn sampler(&self, seed: i64) -> Arc<dyn DensitySampler> {
        Arc::new(ModDensitySampler {
            id: self.id.clone(),
            function: self.function.clone(),
            seed,
        })
    }
}
impl std::fmt::Debug for ModDensity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModDensity")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// A mod node bound to one world's seed: a volume is one call of the mod's `fill`.
struct ModDensitySampler {
    id: String,
    function: Arc<dyn ModDensityFunction>,
    seed: i64,
}
impl DensitySampler for ModDensitySampler {
    fn sample_value(&self, _: &mut SamplerContext, position: [i32; 3]) -> Result<f32, SampleError> {
        Ok(self.function.sample(position, self.seed))
    }
    fn sample_volume(
        &self,
        _: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        let out = buffer
            .get_mut(..volume.size())
            .ok_or(DensityError::BufferTooShort)?;
        self.function.fill(
            volume.minimum(),
            volume.steps(),
            volume.sizes(),
            self.seed,
            out,
        );
        Ok(())
    }
    /// Each value depends on its own position (and the fixed seed) only, so a lattice sampled
    /// once can be reused, as vanilla's noises are.
    fn pointwise_inputs(&self) -> Option<Vec<&Arc<dyn DensitySampler>>> {
        Some(Vec::new())
    }
}
impl std::fmt::Debug for ModDensitySampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModDensitySampler")
            .field("id", &self.id)
            .field("seed", &self.seed)
            .finish_non_exhaustive()
    }
}
