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

/// A decoded node: a pure function of the world seed and the block position. `Err` is the mod's
/// code failing (the adapter's contained panic): the world's [`ModDensityFailure`] records it,
/// what reads the value fails with [`SampleError::External`], and the world's session ends.
pub trait ModDensityFunction: Send + Sync {
    /// `[min, max]` of every value, read once when the node decodes.
    fn range(&self) -> [f32; 2];
    /// `false`: the value never depends on y. Read once when the node decodes.
    fn varies_with_y(&self) -> bool;
    fn sample(&self, position: [i32; 3], seed: i64) -> Result<f32, String>;
    /// `count` positions from `origin`, `step` apart, into `out` (exactly that many values, y
    /// fastest, then x, then z).
    fn fill(
        &self,
        origin: [i32; 3],
        step: [i32; 3],
        count: [usize; 3],
        seed: i64,
        out: &mut [f32],
    ) -> Result<(), String>;
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

/// The first failure of a mod density function in one world (one density registry, so one
/// dimension's noise settings): a failure is fatal to the world's session, and once there is one
/// no mod function of that registry is called again. Its text names the type and the mod.
#[derive(Debug, Default)]
pub struct ModDensityFailure(OnceLock<String>);
impl ModDensityFailure {
    pub fn get(&self) -> Option<&str> {
        self.0.get().map(String::as_str)
    }
    fn record(&self, id: &str, error: &str) {
        let mut first = false;
        let text = self.0.get_or_init(|| {
            first = true;
            let namespace = id.split_once(':').map_or(id, |(namespace, _)| namespace);
            format!("density function type {id} of mod {namespace} failed: {error}")
        });
        if first {
            log::error!("{text}");
        }
    }
}

/// A decoded mod node in the graph, waiting for the compiler's seed.
pub(super) struct ModDensity {
    pub(super) id: String,
    pub(super) function: Arc<dyn ModDensityFunction>,
    pub(super) failure: Arc<ModDensityFailure>,
}
impl SeededDensity for ModDensity {
    fn sampler(&self, seed: i64) -> Arc<dyn DensitySampler> {
        Arc::new(ModDensitySampler {
            id: self.id.clone(),
            function: self.function.clone(),
            failure: self.failure.clone(),
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
    failure: Arc<ModDensityFailure>,
    seed: i64,
}
impl DensitySampler for ModDensitySampler {
    fn sample_value(&self, _: &mut SamplerContext, position: [i32; 3]) -> Result<f32, SampleError> {
        self.not_failed()?;
        self.function
            .sample(position, self.seed)
            .map_err(|error| self.failed(&error))
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
        self.not_failed()?;
        self.function
            .fill(
                volume.minimum(),
                volume.steps(),
                volume.sizes(),
                self.seed,
                out,
            )
            .map_err(|error| self.failed(&error))
    }
    /// Each value depends on its own position (and the fixed seed) only, so a lattice sampled
    /// once can be reused, as vanilla's noises are.
    fn pointwise_inputs(&self) -> Option<Vec<&Arc<dyn DensitySampler>>> {
        Some(Vec::new())
    }
}
impl ModDensitySampler {
    /// After a failure in this world, no mod function is called again: the session is ending.
    fn not_failed(&self) -> Result<(), SampleError> {
        match self.failure.get() {
            Some(_) => Err(SampleError::External),
            None => Ok(()),
        }
    }
    /// The mod's failure, recorded for the world, as the error the reader gets.
    fn failed(&self, error: &str) -> SampleError {
        self.failure.record(&self.id, error);
        SampleError::External
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
