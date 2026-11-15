//! Custom item textures' enchantment glints: the layers a stack draws over (or instead of)
//! vanilla's glint, and how strongly each draws over time. Which layers a stack gets is the pack
//! format's ([`crate::cit::Matcher::glint`]); a layer's look is its rule's [`crate::cit::Glint`].

/// One glint layer a stack draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlintLayer {
    /// The rule, an index into `CitRules::rules`.
    pub rule: usize,
    /// Its `average` or `layered` share, 1 under `none` and `cycle`.
    pub weight: f32,
    /// Its `duration`, for `cycle`.
    duration: f32,
}

impl GlintLayer {
    /// Rule `rule`'s layer at `weight`, on for `duration` seconds when the layers take turns.
    pub fn new(rule: usize, weight: f32, duration: f32) -> Self {
        Self {
            rule,
            weight,
            duration,
        }
    }
}

/// What a stack's glint is made of, worked out when the stack changes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StackGlint {
    /// Bottom first (`layer` ascending).
    pub layers: Vec<GlintLayer>,
    /// Whether vanilla's glint draws (on a stack that has one) under the layers.
    pub vanilla: bool,
    /// `fade` when the layers take turns (`method=cycle`).
    cycle_fade: Option<f32>,
}

impl StackGlint {
    /// `layers`, bottom first; `vanilla` whether vanilla's glint draws under them; `cycle_fade`
    /// the seconds each layer fades into the next when they take turns, else `None` (each
    /// draws at its weight).
    pub fn from_parts(layers: Vec<GlintLayer>, vanilla: bool, cycle_fade: Option<f32>) -> Self {
        Self {
            layers,
            vanilla,
            cycle_fade,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }

    /// Layer `index`'s intensity `seconds` into the game: its weight, and under `method=cycle`
    /// its turn.
    pub fn intensity(&self, index: usize, seconds: f64) -> f32 {
        let layer = self.layers[index];
        match self.cycle_fade {
            None => layer.weight,
            Some(fade) => cycle_intensity(&self.layers, fade, index, seconds),
        }
    }
}

/// CIT Resewn's `Loops.statelessFadingLoop`: each layer shows alone for its `duration`, then
/// fades into the next over `fade` seconds, the last into the first. With no time at all
/// (every `duration` and `fade` 0) the bottom layer shows alone.
fn cycle_intensity(layers: &[GlintLayer], fade: f32, index: usize, seconds: f64) -> f32 {
    let n = layers.len();
    let fade = f64::from(fade);
    let period: f64 = layers.iter().map(|l| f64::from(l.duration) + fade).sum();
    if n == 1 || period <= 0.0 {
        return if index == 0 { 1.0 } else { 0.0 };
    }
    let mut t = seconds.rem_euclid(period);
    for (i, layer) in layers.iter().enumerate() {
        let duration = f64::from(layer.duration);
        if t < duration {
            return if i == index { 1.0 } else { 0.0 };
        }
        t -= duration;
        // Defensive: the last fade also takes anything rounding might leave of the period
        // (none seen), ending fully on the first layer.
        if t < fade || i + 1 == n {
            let into = if fade > 0.0 {
                (t / fade).min(1.0) as f32
            } else {
                1.0
            };
            return if i == index {
                1.0 - into
            } else if (i + 1) % n == index {
                into
            } else {
                0.0
            };
        }
        t -= fade;
    }
    // Not reached, as the last layer returns above; no layer rather than a panic.
    0.0
}
