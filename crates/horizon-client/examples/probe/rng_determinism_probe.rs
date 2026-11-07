//! Rust side of tools/test_rng_determinism_parity.py: the same seed sequence, and draws from it,
//! as RngDeterminismOracle. `HORIZON_RNG_SEED` (set by the harness) must make
//! `horizon_core::random::unique_seed()` repeatable; `RandomSource.create()` is Horizon's
//! `LegacyRandom::new(unique_seed())`.
//!
//! Arguments: the input file holding the count, then the output file.
use horizon_core::random::{LegacyRandom, RandomSource, XoroshiroRandom, unique_seed};
use std::{error::Error, fmt::Write, path::PathBuf};

pub fn main() -> Result<(), Box<dyn Error>> {
    // `crate::args()` without naming it, so tools/parity.py can narrow this
    // probe's Rust scope to horizon-core.
    let mut args: Vec<String> = std::env::args().collect();
    if args.len() > 1 {
        args.remove(1);
    }
    let input = PathBuf::from(args.get(1).ok_or("missing input path")?);
    let output = PathBuf::from(args.get(2).ok_or("missing output path")?);
    let count: usize = std::fs::read_to_string(input)?.trim().parse()?;
    let mut text = String::new();
    for i in 0..count {
        let seed = unique_seed();
        writeln!(text, "seed {i} {seed}")?;
        writeln!(text, "legacy {i} {}", LegacyRandom::new(seed).next_long())?;
        writeln!(
            text,
            "xoroshiro {i} {}",
            XoroshiroRandom::new(seed).next_long()
        )?;
    }
    for i in 0..count {
        let seed = unique_seed();
        writeln!(text, "create {i} {}", LegacyRandom::new(seed).next_long())?;
    }
    let seed = unique_seed();
    writeln!(text, "threadlocal {}", LegacyRandom::new(seed).next_long())?;
    for name in ["title_splash", "structure_block", "firework_spark"] {
        let seed = horizon_core::random::pinned_seed(name).ok_or("HORIZON_RNG_SEED is not set")?;
        writeln!(text, "named {name} {seed}")?;
    }
    std::fs::write(output, text)?;
    Ok(())
}
