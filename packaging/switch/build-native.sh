#!/usr/bin/env bash
# The Switch build without a container, on Linux or WSL2 (docs/switch.md): the same steps as
# build.sh nro, run directly. Needs devkitA64 + libnx in /opt/devkitpro, a nightly Rust with
# rust-src, and a bevy-horizon checkout (BEVY_HORIZON, default ../bevy-horizon) with NVK built
# (linux/setup-toolchain.sh and linux/build-nvk.sh do both).
#   packaging/switch/build-native.sh nro      horizon-client -> horizon.nro
#   HORIZON_NVK_LIB=skip ... nro              compile only (no NVK needed; the link fails)
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BH="$(cd "${BEVY_HORIZON:-$ROOT/../bevy-horizon}" && pwd)"
export DEVKITPRO=/opt/devkitpro DEVKITARM=/opt/devkitpro/devkitA64
export PATH="/usr/local/cargo/bin:/opt/devkitpro/devkitA64/bin:/opt/devkitpro/tools/bin:$PATH"
export RUSTUP_HOME="${RUSTUP_HOME:-/usr/local/rustup}" CARGO_HOME="${CARGO_HOME:-/usr/local/cargo}"
export HORIZON_NVK_LIB="${HORIZON_NVK_LIB:-$BH/nvk/lib}"
CONFIG="--config $BH/.cargo/config.toml --config $ROOT/packaging/switch/cargo-bevy-horizon.toml --config $ROOT/packaging/switch/cargo-patch.toml"
OUT=target/aarch64-nintendo-horizon/release
FEATURES="${FEATURES-audio}"
cd "$ROOT"
nro() {
    cp Cargo.lock Cargo.lock.switch-saved
    trap 'mv -f "$ROOT/Cargo.lock.switch-saved" "$ROOT/Cargo.lock"' EXIT
    STD_LIB="$(rustc --print sysroot)/lib/rustlib/src/rust/library"
    cargo update $CONFIG -p libc --manifest-path "$STD_LIB/Cargo.toml"
    if grep -A2 '^name = "libc"' "$STD_LIB/Cargo.lock" | grep -q '^source = '; then
        echo "error: std would build against crates.io libc, not bevy-horizon patches/libc" >&2
        exit 1
    fi
    cargo build --release ${JOBS:+-j "$JOBS"} $CONFIG \
        -Z build-std=core,alloc,std,panic_abort -Z json-target-spec \
        --target "$BH/aarch64-nintendo-horizon.json" \
        -p horizon-client --bin horizon-client --no-default-features ${FEATURES:+--features "$FEATURES"} ${CARGO_ARGS:-}
    nacptool --create "${TITLE:-Horizon}" "${AUTHOR:-Horizon contributors}" "${VERSION:-0.1.0}" "$OUT/horizon-client.nacp"
    elf2nro "$OUT/horizon-client" "$OUT/horizon.nro" --nacp="$OUT/horizon-client.nacp" \
        --icon="${ICON:-/opt/devkitpro/libnx/default_icon.jpg}" ${ROMFS:+--romfsdir="$ROMFS"}
    echo "done: $ROOT/$OUT/horizon.nro"
}
case "${1:-}" in
    nro) nro ;;
    *) sed -n '2,6p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2; exit 1 ;;
esac
