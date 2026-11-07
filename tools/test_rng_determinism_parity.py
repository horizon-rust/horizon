#!/usr/bin/env python3
"""Differential test for deterministic RNG: one master seed yields one sequence.

The Java oracle (`RngDeterminismOracle`, run with the `rng-agent` compiled by
tools/rng_determinism.py) and the Rust probe (`rng_determinism_probe`) both create
the same number of unseeded streams and draw from each with `LegacyRandom` and
`XoroshiroRandom`. The outputs and the creation traces must match, which proves
`RandomSupport.generateUniqueSeed()` and `horizon_core::random::unique_seed()`
share a sequence, and that the two patched `RandomSource` entry points consume it
the same way.

  python3 tools/rng_determinism.py verify --trace
"""
try:
    from . import rng_determinism
except ImportError:
    import rng_determinism


def main():
    rng_determinism.verify(trace=True)


if __name__ == '__main__':
    main()
