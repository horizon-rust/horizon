//! Master-seeded determinism for parity runs.
//!
//! Vanilla's unseeded streams come from `RandomSupport.generateUniqueSeed()`:
//! `uniquifier.updateAndGet(x -> x * STEP) ^ System.nanoTime()`. The clock is
//! the only nondeterministic term. With `HORIZON_RNG_SEED` (or
//! `HORIZON_STILL=1`) set, that clock term is the master seed instead, so a
//! Horizon run becomes repeatable. This does not make Horizon match vanilla:
//! the two need not create or consume streams in the same order. When a call
//! site's order differs (particles, for example), give it a name with
//! [`pinned_seed`] instead of taking the next value of the shared sequence.

use super::seed_from_hash;
use std::io::Write;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// `RandomSupport.SEED_UNIQUIFIER` and its multiplier.
pub const UNIQUIFIER_START: i64 = 8_682_522_807_148_012;
pub const UNIQUIFIER_STEP: i64 = 1_181_783_497_276_652_981;

static UNIQUIFIER: AtomicI64 = AtomicI64::new(UNIQUIFIER_START);
static INDEX: AtomicU64 = AtomicU64::new(0);

/// The seed every unseeded stream and every named stream derives from.
///
/// `HORIZON_RNG_SEED` wins (decimal, `0x` hex, or a leading `-`); otherwise a
/// still run is deterministic too (`HORIZON_STILL=1`, like screenshots).
pub fn master_seed() -> Option<i64> {
    static MASTER: OnceLock<Option<i64>> = OnceLock::new();
    *MASTER.get_or_init(|| {
        if let Ok(text) = std::env::var("HORIZON_RNG_SEED")
            && !text.trim().is_empty()
        {
            return parse_seed(&text);
        }
        std::env::var("HORIZON_STILL")
            .is_ok_and(|value| value == "1")
            .then_some(0)
    })
}

/// `RandomSupport.generateUniqueSeed`: the next unseeded stream's seed.
///
/// Without a master seed this is the clock-based stream. With one, the clock
/// term is the master seed instead, making the run repeatable.
pub fn unique_seed() -> i64 {
    let next = next_uniquifier(&UNIQUIFIER);
    let seed = match master_seed() {
        Some(master) => next ^ master,
        None => next ^ clock_nanos(),
    };
    if trace_path().is_some() {
        record(INDEX.fetch_add(1, Ordering::Relaxed), seed);
    }
    seed
}

/// A seed named by `name`, stable across runs and independent of creation order.
///
/// `name` is hashed the way vanilla's `RandomSupport.seedFromHashOf` hashes a
/// string (MD5, first 64 bits), then XORed with the master seed. Java can build
/// the same value with
/// `RandomSupport.seedFromHashOf(name).seedLo() ^ masterSeed`.
///
/// `None` when no master seed is set: the caller keeps its normal behaviour.
pub fn pinned_seed(name: &str) -> Option<i64> {
    master_seed().map(|master| pinned_seed_with(master, name))
}

fn pinned_seed_with(master: i64, name: &str) -> i64 {
    seed_from_hash(name)[0] as i64 ^ master
}

/// `pinned_seed(name)`, or a fresh unseeded stream when determinism is off.
pub fn seed_for(name: &str) -> i64 {
    pinned_seed(name).unwrap_or_else(unique_seed)
}

fn next_uniquifier(uniquifier: &AtomicI64) -> i64 {
    let mut current = uniquifier.load(Ordering::Relaxed);
    loop {
        let next = current.wrapping_mul(UNIQUIFIER_STEP);
        match uniquifier.compare_exchange_weak(current, next, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => return next,
            Err(actual) => current = actual,
        }
    }
}

fn clock_nanos() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as i64)
}

fn parse_seed(text: &str) -> Option<i64> {
    let text = text.trim().replace('_', "");
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.as_str()),
    };
    let value = match digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        Some(hex) => i64::from_str_radix(hex, 16)
            .or_else(|_| u64::from_str_radix(hex, 16).map(|v| v as i64))
            .ok()?,
        None => digits.parse::<i64>().ok()?,
    };
    Some(if negative {
        value.wrapping_neg()
    } else {
        value
    })
}

/// `HORIZON_RNG_TRACE`: one line per unseeded stream, `create INDEX SEED SITE`.
fn trace_path() -> Option<&'static std::path::PathBuf> {
    static PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    PATH.get_or_init(|| {
        let path = std::env::var_os("HORIZON_RNG_TRACE")?;
        if path.is_empty() {
            return None;
        }
        let path = std::path::PathBuf::from(path);
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            let _ = std::fs::create_dir_all(parent);
        }
        Some(path)
    })
    .as_ref()
}

fn record(index: u64, seed: i64) {
    static WRITER: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();
    let file = WRITER.get_or_init(|| {
        trace_path()
            .and_then(|path| std::fs::File::create(path).ok())
            .map(Mutex::new)
    });
    if let Some(file) = file
        && let Ok(mut file) = file.lock()
    {
        let _ = writeln!(file, "create {index} {seed} {}", caller());
    }
}

/// The first frame outside the RNG plumbing, as `crate::module::function`.
fn caller() -> String {
    let text = std::backtrace::Backtrace::force_capture().to_string();
    let mut name = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some((frame, function)) = line.split_once(": ")
            && frame.chars().all(|c| c.is_ascii_digit())
        {
            name.clear();
            name.push_str(function);
        } else if line.starts_with("at ") && !name.is_empty() {
            let file = line[3..].rsplit('/').next().unwrap_or(&line[3..]);
            let file = file.split(':').next().unwrap_or(file);
            if file != "determinism.rs" && is_call_site(&name) {
                return format!("{name} ({file})");
            }
        }
    }
    name
}

fn is_call_site(name: &str) -> bool {
    // Release builds shorten some inlined symbols to a bare function name.
    name.contains("::")
        && !name.starts_with("core::")
        && !name.starts_with("std::")
        && !name.contains("backtrace")
        && !name.starts_with("horizon_core::random::determinism")
        && !name.ends_with("::unique_seed")
        && !name.ends_with("::entity_random")
        && !name.ends_with("::caller")
        && !name.ends_with("::record")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_stream_is_stable_and_mastered() {
        let a = pinned_seed_with(42, "block_breaking_particle");
        let b = pinned_seed_with(42, "block_breaking_particle");
        assert_eq!(a, b);
        assert_ne!(
            pinned_seed_with(42, "block_breaking_particle"),
            pinned_seed_with(42, "spectator_team_icon")
        );
        assert_ne!(
            pinned_seed_with(7, "block_breaking_particle"),
            pinned_seed_with(42, "block_breaking_particle")
        );
    }

    /// Values from the Java agent with `-Dhorizon.rng.seed=42`, independently
    /// derived from `(uniq * STEP) ^ 42`.
    #[test]
    fn seeded_sequence_matches_vanillas_uniquifier_formula() {
        let uniquifier = AtomicI64::new(UNIQUIFIER_START);
        let seeds: Vec<i64> = (0..3).map(|_| next_uniquifier(&uniquifier) ^ 42).collect();
        assert_eq!(
            seeds,
            [
                3_447_679_086_515_839_990,
                -2_942_033_378_085_796_186,
                9_105_146_733_113_736_406,
            ]
        );
    }

    #[test]
    fn seeded_sequence_matches_for_seed_zero() {
        let uniquifier = AtomicI64::new(UNIQUIFIER_START);
        let seeds: Vec<i64> = (0..2).map(|_| next_uniquifier(&uniquifier)).collect();
        assert_eq!(
            seeds,
            [3_447_679_086_515_839_964, -2_942_033_378_085_796_212]
        );
    }

    #[test]
    fn seed_text_accepts_decimal_hex_and_negative() {
        assert_eq!(parse_seed("42"), Some(42));
        assert_eq!(parse_seed("0x2a"), Some(42));
        assert_eq!(parse_seed("-7"), Some(-7));
        assert_eq!(parse_seed("1_000"), Some(1000));
        assert_eq!(parse_seed("nope"), None);
    }

    /// `RandomSupport.seedFromHashOf(name).seedLo() ^ 42`, printed by Java.
    #[test]
    fn named_streams_match_vanillas_seed_from_hash() {
        for (name, expected) in [
            ("title_splash", -1_413_213_973_635_151_920),
            ("structure_block", 6_153_619_978_476_351_403),
            ("command_item_modify", 4_249_450_875_184_394_523),
            ("spectator_team_icon", 93_995_956_229_091_896),
        ] {
            assert_eq!(pinned_seed_with(42, name), expected, "{name}");
        }
    }
}
