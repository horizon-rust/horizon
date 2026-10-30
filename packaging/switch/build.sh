#!/usr/bin/env bash
# Builds Horizon for the Nintendo Switch (an .nro) in a container: docs/switch.md.
#
#   packaging/switch/build.sh image   the toolchain image (devkitA64, libnx, nightly Rust)
#   packaging/switch/build.sh nvk     NVK, Mesa's Vulkan driver for the Switch's GPU (slow, once)
#   packaging/switch/build.sh nro     horizon-client -> target/aarch64-nintendo-horizon/release/horizon.nro
#   packaging/switch/build.sh all     the three, in order
#
# Needs Docker or Podman on Linux (or WSL2): CONTAINER_ENGINE picks one. It uses two checkouts that
# it clones beside this repository if they are missing:
#   ../bevy-horizon   https://github.com/JohnUzoka/bevy-horizon (MIT OR Apache-2.0): the target spec,
#                     the crates it patches for Horizon, and the NVK build scripts
#   ../mesa-switch    https://github.com/danfromtico/mesa-switch (only for `nvk`)
# Variables: JOBS (cargo's parallel jobs; each rustc can take 1-2 GB of RAM), CARGO_ARGS (more
# `cargo build` arguments), FEATURES (horizon-client features, `audio` by default: cpal on libnx's
# audio renderer; no gilrs, no mods), VERSION, AUTHOR, ICON (a 256x256 JPEG for the HOME menu).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BEVY_HORIZON="${BEVY_HORIZON:-$ROOT/../bevy-horizon}"
MESA_SWITCH="${MESA_SWITCH:-$ROOT/../mesa-switch}"
# The revisions this port was written against.
BEVY_HORIZON_REV=3eac779b232ee47cd0eb957676c64386ac7b0f1d
MESA_SWITCH_REV=606441c90e459028a50fca352daebfc20211f890
IMAGE=horizon-switch-toolchain:local
ENGINE="${CONTAINER_ENGINE:-$(command -v docker >/dev/null && echo docker || echo podman)}"
VERSION="${VERSION:-0.1.0}"
AUTHOR="${AUTHOR:-Horizon contributors}"
TITLE="${TITLE:-Horizon}"
BIN=horizon-client
OUT=target/aarch64-nintendo-horizon/release

clone() { # dir url rev
    if [ ! -d "$1/.git" ]; then
        echo "==> cloning $2 into $1"
        git clone "$2" "$1"
        git -C "$1" checkout --quiet "$3"
    fi
}

image() {
    clone "$BEVY_HORIZON" https://github.com/JohnUzoka/bevy-horizon.git "$BEVY_HORIZON_REV"
    echo "==> building $IMAGE"
    # bevy-horizon's NVK scripts look for this tag.
    "$ENGINE" build -t "$IMAGE" -t bevy-horizon-toolchain:local -f "$ROOT/packaging/switch/Containerfile" "$ROOT/packaging/switch"
}

need_image() { "$ENGINE" image inspect "$IMAGE" >/dev/null 2>&1 || image; }

nvk() {
    need_image
    clone "$MESA_SWITCH" https://github.com/danfromtico/mesa-switch.git "$MESA_SWITCH_REV"
    (cd "$BEVY_HORIZON" && scripts/build_nvk.sh "$MESA_SWITCH" && scripts/localize_nvk.sh)
}

nro() {
    need_image
    clone "$BEVY_HORIZON" https://github.com/JohnUzoka/bevy-horizon.git "$BEVY_HORIZON_REV"
    if [ ! -f "$BEVY_HORIZON/nvk/lib/libvulkan_bh.a" ]; then
        echo "error: $BEVY_HORIZON/nvk/lib/libvulkan_bh.a is missing: run packaging/switch/build.sh nvk first" >&2
        exit 1
    fi
    if [ -n "${ICON:-}" ]; then
        ICON="$(cd "$(dirname "$ICON")" && pwd)/$(basename "$ICON")"
    fi
    BEVY_HORIZON="$(cd "$BEVY_HORIZON" && pwd)"
    local config="--config /src/bevy-horizon/.cargo/config.toml"
    config="$config --config /src/horizon-nx/packaging/switch/cargo-bevy-horizon.toml"
    config="$config --config /src/horizon-nx/packaging/switch/cargo-patch.toml"
    local mounts=(-v "$ROOT:/src/horizon-nx:z" -v "$BEVY_HORIZON:/src/bevy-horizon:z" -v bh-cargo-registry:/usr/local/cargo/registry)

    echo "==> compiling $BIN for the Switch (Horizon OS)"
    # The patches make cargo edit Cargo.lock; it is put back afterwards, so desktop builds keep theirs.
    cp "$ROOT/Cargo.lock" "$ROOT/Cargo.lock.switch-saved"
    trap 'mv -f "$ROOT/Cargo.lock.switch-saved" "$ROOT/Cargo.lock"' EXIT
    "$ENGINE" run --rm --log-driver=none \
        -e CONFIG="$config" -e JOBS="${JOBS:-}" -e CARGO_ARGS="${CARGO_ARGS:-}" -e FEATURES="${FEATURES-audio}" \
        -e HORIZON_NVK_LIB=/src/bevy-horizon/nvk/lib \
        "${mounts[@]}" -w /src/horizon-nx "$IMAGE" \
        bash -euo pipefail -c '
            # std is its own workspace and does not see our [patch]; a config-file patch applies to
            # it, but its Cargo.lock must move to the patched libc first.
            STD_LIB="$(rustc --print sysroot)/lib/rustlib/src/rust/library"
            cargo update $CONFIG -p libc --manifest-path "$STD_LIB/Cargo.toml"
            if grep -A2 "^name = \"libc\"" "$STD_LIB/Cargo.lock" | grep -q "^source = "; then
                echo "error: std would build against crates.io libc, not bevy-horizon patches/libc" >&2
                exit 1
            fi
            # A patch cargo does not use means the crates.io crate is built, which cannot work here.
            if cargo metadata $CONFIG --format-version 1 --filter-platform aarch64-unknown-none 2>&1 >/dev/null | grep -i "patch.*was not used"; then
                echo "error: a patch is not used: lock its version in Cargo.lock (cargo update --precise)" >&2
                exit 1
            fi
            cargo build --release ${JOBS:+-j "$JOBS"} $CONFIG \
                -Z build-std=core,alloc,std,panic_abort -Z json-target-spec \
                --target /src/bevy-horizon/aarch64-nintendo-horizon.json \
                -p horizon-client --bin '"$BIN"' --no-default-features ${FEATURES:+--features "$FEATURES"} $CARGO_ARGS'

    echo "==> packaging horizon.nro"
    local icon_mount=()
    [ -n "${ICON:-}" ] && icon_mount=(-v "$ICON:/src/icon.jpg:ro,z")
    "$ENGINE" run --rm --log-driver=none \
        -e BIN="$BIN" -e TITLE="$TITLE" -e AUTHOR="$AUTHOR" -e VERSION="$VERSION" -e OUT="$OUT" \
        -e ICON="${ICON:+/src/icon.jpg}" -e ROMFS="${ROMFS:-}" \
        "${mounts[@]}" "${icon_mount[@]}" -w /src/horizon-nx "$IMAGE" \
        sh -c 'set -e
            nacptool --create "$TITLE" "$AUTHOR" "$VERSION" "$OUT/$BIN.nacp"
            elf2nro "$OUT/$BIN" "$OUT/horizon.nro" --nacp="$OUT/$BIN.nacp" \
                --icon="${ICON:-/opt/devkitpro/libnx/default_icon.jpg}" ${ROMFS:+--romfsdir="$ROMFS"}'

    echo "==> checking the NRO's asset section (sphaira refuses an NRO without one)"
    python3 - "$ROOT/$OUT/horizon.nro" <<'PY'
import os, struct, sys
path = sys.argv[1]
with open(path, "rb") as f:
    head = f.read(0x80)
    image = struct.unpack_from("<I", head, 0x18)[0]
    f.seek(image)
    aset = f.read(0x38)
size = os.path.getsize(path)
if head[0x10:0x14] != b"NRO0" or aset[:4] != b"ASET" or size < image + 0x80:
    sys.exit(f"error: {path} has no asset section (image 0x{image:x}, file 0x{size:x})")
print(f"    ok: image 0x{image:x} + ASET section, file {size / 1048576:.1f} MiB")
PY
    echo "==> done: $ROOT/$OUT/horizon.nro (copy it to switch/horizon/ on the SD card; docs/switch.md)"
}

case "${1:-}" in
    image) image ;;
    nvk) nvk ;;
    nro) nro ;;
    all) image && nvk && nro ;;
    *) sed -n '2,17p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2; exit 1 ;;
esac
