#!/bin/sh
# Fastest rebuilds. Workspace profile "fast", unchanged:
# incremental on, no cross-crate LTO, 16 codegen units.
# CPU stays the portable default. The native release build is tools/build-runtime.sh.
set -eu
cd "$(dirname "$0")/.."
unset RUSTFLAGS CARGO_ENCODED_RUSTFLAGS
echo "$(rustc --version)"
echo "dev: profile fast, incremental, lto off, codegen-units 16"
cargo build --locked --profile fast -p horizon-client -p horizon-prepare
echo "binary: target/fast/horizon-client"
echo "prepare: target/fast/horizon-prepare"
