use super::{DensityError, DensityVolume, NanRange, PerlinNoise, SimplexNoise, SmearedPerlinNoise};
use std::sync::Arc;

/// Immutable noise sampling. Implementations may override the default scalar
/// volume traversal when vanilla has a distinct bulk arithmetic path.
pub trait Noise: std::fmt::Debug + Send + Sync {
    fn range(&self) -> Result<[f32; 2], NanRange>;
    fn get2(&self, x: f64, z: f64) -> f32;
    fn get3(&self, x: f64, y: f64, z: f64) -> f32;
    fn add_to_volume(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) -> Result<(), DensityError> {
        if buffer.len() < volume.size() {
            return Err(DensityError::BufferTooShort);
        }
        let [sx, sy, sz] = volume.sizes();
        let mut index = 0;
        for z in 0..sz {
            let z = f64::from(volume.coordinate(2, z)) * xz_scale;
            for x in 0..sx {
                let x = f64::from(volume.coordinate(0, x)) * xz_scale;
                for y in 0..sy {
                    let y = f64::from(volume.coordinate(1, y)) * y_scale;
                    buffer[index] += amplitude * self.get3(x, y, z);
                    index += 1;
                }
            }
        }
        Ok(())
    }
    /// [`Noise::add_to_volume`] for a caller that reads only the Ys set in `needed`: bit `y` of
    /// `needed[column]`, X/Z columns in buffer order. The other Ys may be left unchanged.
    fn add_to_needed(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
        _needed: &[u64],
    ) -> Result<(), DensityError> {
        self.add_to_volume(buffer, volume, xz_scale, y_scale, amplitude)
    }
    #[doc(hidden)]
    fn as_perlin(&self) -> Option<&PerlinNoise> {
        None
    }
    #[doc(hidden)]
    fn as_smeared_perlin(&self) -> Option<&SmearedPerlinNoise> {
        None
    }
    #[doc(hidden)]
    fn as_simplex(&self) -> Option<&SimplexNoise> {
        None
    }
}

impl Noise for PerlinNoise {
    fn range(&self) -> Result<[f32; 2], NanRange> {
        Ok(PerlinNoise::range(self))
    }
    fn get2(&self, x: f64, z: f64) -> f32 {
        PerlinNoise::get2(self, x, z)
    }
    fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        PerlinNoise::get3(self, x, y, z)
    }
    fn add_to_volume(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) -> Result<(), DensityError> {
        PerlinNoise::add_to_volume(self, buffer, volume, xz_scale, y_scale, amplitude)
    }
    fn add_to_needed(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
        needed: &[u64],
    ) -> Result<(), DensityError> {
        PerlinNoise::add_to_needed(self, buffer, volume, xz_scale, y_scale, amplitude, needed)
    }
    fn as_perlin(&self) -> Option<&PerlinNoise> {
        Some(self)
    }
}
impl Noise for SmearedPerlinNoise {
    fn range(&self) -> Result<[f32; 2], NanRange> {
        SmearedPerlinNoise::range(self)
    }
    fn get2(&self, x: f64, z: f64) -> f32 {
        SmearedPerlinNoise::get2(self, x, z)
    }
    fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        SmearedPerlinNoise::get3(self, x, y, z)
    }
    fn add_to_volume(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) -> Result<(), DensityError> {
        SmearedPerlinNoise::add_to_volume(self, buffer, volume, xz_scale, y_scale, amplitude)
    }
    fn add_to_needed(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
        needed: &[u64],
    ) -> Result<(), DensityError> {
        SmearedPerlinNoise::add_to_needed(
            self, buffer, volume, xz_scale, y_scale, amplitude, needed,
        )
    }
    fn as_smeared_perlin(&self) -> Option<&SmearedPerlinNoise> {
        Some(self)
    }
}
impl Noise for SimplexNoise {
    fn range(&self) -> Result<[f32; 2], NanRange> {
        Ok(SimplexNoise::range(self))
    }
    fn get2(&self, x: f64, z: f64) -> f32 {
        SimplexNoise::get2(self, x, z)
    }
    fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        SimplexNoise::get3(self, x, y, z)
    }
    fn as_simplex(&self) -> Option<&SimplexNoise> {
        Some(self)
    }
}

#[derive(Clone, Debug)]
struct PerlinLayer {
    noise: PerlinNoise,
    frequency: f64,
    amplitude: f32,
}

#[derive(Clone, Debug)]
struct SmearedPerlinLayer {
    noise: SmearedPerlinNoise,
    frequency: f64,
    amplitude: f32,
}

#[derive(Clone, Debug)]
struct SimplexLayer {
    noise: SimplexNoise,
    frequency: f64,
    amplitude: f32,
}

/// Homogeneous devirtualized layer storage for contiguous cache-friendly sampling
/// without per-layer dynamic dispatch.
#[derive(Clone, Debug)]
enum FastLayers {
    Perlin(Box<[PerlinLayer]>),
    SmearedPerlin(Box<[SmearedPerlinLayer]>),
    Simplex(Box<[SimplexLayer]>),
    Dynamic,
}

#[derive(Clone, Debug)]
pub struct NoiseLayer {
    pub noise: Arc<dyn Noise>,
    pub frequency: f64,
    pub amplitude: f32,
}

#[derive(Clone, Debug)]
pub struct NoiseStack {
    layers: Vec<NoiseLayer>,
    fast: FastLayers,
    finite_frequencies: bool,
    range: [f32; 2],
}

impl NoiseStack {
    pub fn builder() -> NoiseStackBuilder {
        NoiseStackBuilder::default()
    }
    pub fn layers(&self) -> &[NoiseLayer] {
        &self.layers
    }
    pub fn get_layer(&self, index: usize) -> Option<&Arc<dyn Noise>> {
        self.layers.get(index).map(|layer| &layer.noise)
    }
}
impl Noise for NoiseStack {
    fn range(&self) -> Result<[f32; 2], NanRange> {
        Ok(self.range)
    }
    fn get2(&self, x: f64, z: f64) -> f32 {
        let mut value = 0.0;
        match &self.fast {
            FastLayers::Perlin(layers) => {
                for layer in layers {
                    value += layer.amplitude
                        * layer.noise.get2(x * layer.frequency, z * layer.frequency);
                }
            }
            FastLayers::SmearedPerlin(layers) => {
                for layer in layers {
                    value += layer.amplitude
                        * layer.noise.get2(x * layer.frequency, z * layer.frequency);
                }
            }
            FastLayers::Simplex(layers) => {
                for layer in layers {
                    value += layer.amplitude
                        * layer.noise.get2(x * layer.frequency, z * layer.frequency);
                }
            }
            FastLayers::Dynamic => {
                for layer in &self.layers {
                    value += layer.amplitude
                        * layer.noise.get2(x * layer.frequency, z * layer.frequency);
                }
            }
        }
        value
    }
    fn get3(&self, x: f64, y: f64, z: f64) -> f32 {
        let mut value = 0.0;
        match &self.fast {
            FastLayers::Perlin(layers) => {
                if y == 0.0 && self.finite_frequencies {
                    for layer in layers {
                        value += layer.amplitude
                            * layer
                                .noise
                                .get3_y0(x * layer.frequency, z * layer.frequency);
                    }
                } else {
                    for layer in layers {
                        value += layer.amplitude
                            * layer.noise.get3(
                                x * layer.frequency,
                                y * layer.frequency,
                                z * layer.frequency,
                            );
                    }
                }
            }
            FastLayers::SmearedPerlin(layers) => {
                for layer in layers {
                    value += layer.amplitude
                        * layer.noise.get3(
                            x * layer.frequency,
                            y * layer.frequency,
                            z * layer.frequency,
                        );
                }
            }
            FastLayers::Simplex(layers) => {
                for layer in layers {
                    value += layer.amplitude
                        * layer.noise.get3(
                            x * layer.frequency,
                            y * layer.frequency,
                            z * layer.frequency,
                        );
                }
            }
            FastLayers::Dynamic => {
                for layer in &self.layers {
                    value += layer.amplitude
                        * layer.noise.get3(
                            x * layer.frequency,
                            y * layer.frequency,
                            z * layer.frequency,
                        );
                }
            }
        }
        value
    }
    fn add_to_volume(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
    ) -> Result<(), DensityError> {
        self.add_layers(buffer, volume, [xz_scale, y_scale], amplitude, None)
    }
    fn add_to_needed(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        xz_scale: f64,
        y_scale: f64,
        amplitude: f32,
        needed: &[u64],
    ) -> Result<(), DensityError> {
        self.add_layers(buffer, volume, [xz_scale, y_scale], amplitude, Some(needed))
    }
}
impl NoiseStack {
    fn add_layers(
        &self,
        buffer: &mut [f32],
        volume: DensityVolume,
        [xz_scale, y_scale]: [f64; 2],
        amplitude: f32,
        needed: Option<&[u64]>,
    ) -> Result<(), DensityError> {
        if buffer.len() < volume.size() {
            return Err(DensityError::BufferTooShort);
        }
        match &self.fast {
            FastLayers::Perlin(layers) => {
                for layer in layers {
                    let (xz_scale, y_scale) =
                        (xz_scale * layer.frequency, y_scale * layer.frequency);
                    let amplitude = amplitude * layer.amplitude;
                    match needed {
                        Some(needed) => layer
                            .noise
                            .add_to_needed(buffer, volume, xz_scale, y_scale, amplitude, needed)?,
                        None => layer
                            .noise
                            .add_to_volume(buffer, volume, xz_scale, y_scale, amplitude)?,
                    }
                }
            }
            FastLayers::SmearedPerlin(layers) => {
                for layer in layers {
                    let (xz_scale, y_scale) =
                        (xz_scale * layer.frequency, y_scale * layer.frequency);
                    let amplitude = amplitude * layer.amplitude;
                    match needed {
                        Some(needed) => layer
                            .noise
                            .add_to_needed(buffer, volume, xz_scale, y_scale, amplitude, needed)?,
                        None => layer
                            .noise
                            .add_to_volume(buffer, volume, xz_scale, y_scale, amplitude)?,
                    }
                }
            }
            FastLayers::Simplex(layers) => {
                for layer in layers {
                    let (xz_scale, y_scale) =
                        (xz_scale * layer.frequency, y_scale * layer.frequency);
                    let amplitude = amplitude * layer.amplitude;
                    Noise::add_to_volume(
                        &layer.noise,
                        buffer,
                        volume,
                        xz_scale,
                        y_scale,
                        amplitude,
                    )?;
                }
            }
            FastLayers::Dynamic => {
                for layer in &self.layers {
                    let (xz_scale, y_scale) =
                        (xz_scale * layer.frequency, y_scale * layer.frequency);
                    let amplitude = amplitude * layer.amplitude;
                    match needed {
                        Some(needed) => layer
                            .noise
                            .add_to_needed(buffer, volume, xz_scale, y_scale, amplitude, needed)?,
                        None => layer
                            .noise
                            .add_to_volume(buffer, volume, xz_scale, y_scale, amplitude)?,
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct NoiseStackBuilder {
    layers: Vec<NoiseLayer>,
}
impl NoiseStackBuilder {
    pub fn add(&mut self, noise: Arc<dyn Noise>, frequency: f64, amplitude: f32) -> &mut Self {
        self.layers.push(NoiseLayer {
            noise,
            frequency,
            amplitude,
        });
        self
    }
    /// Flattening rescales each existing layer before subsequent sampling.
    pub fn add_stack(&mut self, stack: &NoiseStack, frequency: f64, amplitude: f32) -> &mut Self {
        self.layers
            .extend(stack.layers.iter().map(|layer| NoiseLayer {
                noise: Arc::clone(&layer.noise),
                frequency: layer.frequency * frequency,
                amplitude: layer.amplitude * amplitude,
            }));
        self
    }
    pub fn build(&self) -> Result<NoiseStack, NanRange> {
        let mut range = [0.0; 2];
        for layer in &self.layers {
            let bounds = layer.noise.range()?;
            if layer.amplitude.is_nan() {
                return Err(NanRange);
            }
            let scaled = if bounds[0].is_nan() || bounds[1].is_nan() {
                [f32::NAN; 2]
            } else {
                // Interval.mulBound defines zero times infinity as positive zero.
                let product = |v: f32| {
                    if v == 0.0 || layer.amplitude == 0.0 {
                        0.0
                    } else {
                        v * layer.amplitude
                    }
                };
                let a = product(bounds[0]);
                let b = product(bounds[1]);
                if a < b {
                    [a, b]
                } else if a > b {
                    [b, a]
                } else if a == 0.0 {
                    [
                        f32::from_bits(a.to_bits() | b.to_bits()),
                        f32::from_bits(a.to_bits() & b.to_bits()),
                    ]
                } else {
                    [a, b]
                }
            };
            range = [range[0] + scaled[0], range[1] + scaled[1]];
            if range[0].is_nan() || range[1].is_nan() {
                range = [f32::NAN; 2];
            }
            // Interval.of interns [0,1], including a supplied negative zero.
            if range[0] == 0.0 && range[1] == 1.0 {
                range[0] = 0.0;
            }
        }
        let finite_frequencies = self.layers.iter().all(|layer| layer.frequency.is_finite());
        let fast = if self.layers.is_empty() {
            FastLayers::Perlin(Box::new([]))
        } else if self.layers.iter().all(|l| l.noise.as_perlin().is_some()) {
            FastLayers::Perlin(
                self.layers
                    .iter()
                    .map(|l| PerlinLayer {
                        noise: l.noise.as_perlin().unwrap().clone(),
                        frequency: l.frequency,
                        amplitude: l.amplitude,
                    })
                    .collect(),
            )
        } else if self
            .layers
            .iter()
            .all(|l| l.noise.as_smeared_perlin().is_some())
        {
            FastLayers::SmearedPerlin(
                self.layers
                    .iter()
                    .map(|l| SmearedPerlinLayer {
                        noise: l.noise.as_smeared_perlin().unwrap().clone(),
                        frequency: l.frequency,
                        amplitude: l.amplitude,
                    })
                    .collect(),
            )
        } else if self.layers.iter().all(|l| l.noise.as_simplex().is_some()) {
            FastLayers::Simplex(
                self.layers
                    .iter()
                    .map(|l| SimplexLayer {
                        noise: l.noise.as_simplex().unwrap().clone(),
                        frequency: l.frequency,
                        amplitude: l.amplitude,
                    })
                    .collect(),
            )
        } else {
            FastLayers::Dynamic
        };
        Ok(NoiseStack {
            layers: self.layers.clone(),
            fast,
            finite_frequencies,
            range,
        })
    }
}
