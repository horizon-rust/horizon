#!/usr/bin/env bash
# Sets up a Debian/Ubuntu (or WSL2) machine to build Horizon for the Switch without a container:
# devkitA64 + libnx 4.12 (devkitPro hosts that refuse some networks are bypassed with a mirror),
# a pinned nightly Rust, and the bevy-horizon checkout. Run as root. Then: build-nvk.sh, build-native.sh.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive SRC="${SRC:-/root/src}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
apt-get update
apt-get install -y build-essential git curl wget ca-certificates python3 python3-pip python3-venv python3-mako python3-yaml python3-packaging meson ninja-build pkg-config bison flex cmake rsync xz-utils bzip2 zstd gnupg libexpat1-dev zlib1g-dev libzstd-dev glslang-tools file unzip bindgen cbindgen
apt-get install -y --no-install-recommends llvm-19-dev libclc-19-dev libllvmspirvlib-19-dev libclang-19-dev clang-19 libclang-cpp19-dev spirv-tools spirv-headers libclang-dev clang libpolly-19-dev
# --- devkitA64 r30, newlib r29, libnx source, switch-tools, zlib (from a mirror of devkitPro packages)
b=https://wii.leseratte10.de/devkitPro
mkdir -p /tmp/dkp && cd /tmp/dkp
get() { f=$(basename "$1" | sed 's/%20/ /g;s/%28/(/g;s/%29/)/g'); [ -s "$f" ] || curl -fsSL -A "Mozilla/5.0" -o "$f" "$b/$1"; tar -C / -xf "$f"; }
get "devkitA64/r30%20%282026-06-10%29/devkita64-binutils-2.46.0-1-linux_x86_64.pkg.tar.zst"
get "devkitA64/r30%20%282026-06-10%29/devkita64-gcc-16.1.0-1-linux_x86_64.pkg.tar.zst"
get "devkitA64/r29%20%282026-01%29/devkita64-newlib-4.6.0.20260123-4-any.pkg.tar.zst"
get "devkitA64/r30%20%282026-06-10%29/devkitA64-r30-1-any.pkg.tar.zst"
get "libnx/libnx-4.9.0-1-any.pkg.tar.zst"
get "switch/switch-tools-1.9.0-1-linux_x86_64.pkg.tar.xz"
get "switch/switch-zlib-1.3.1-1-any.pkg.tar.zst"
install -m755 "$HERE/bin2s" /opt/devkitpro/tools/bin/bin2s
# --- libnx from source: the mirror only has 4.9, and Horizon needs audrenGetFrameEvent (4.10+)
set -e
export DEVKITPRO=/opt/devkitpro DEVKITA64=/opt/devkitpro/devkitA64
export PATH=/opt/devkitpro/devkitA64/bin:/opt/devkitpro/tools/bin:/usr/local/bin:/usr/bin:/bin
mkdir -p /tmp/dkp && cd /tmp/dkp
[ -s rules.zst ] || curl -fsSL -A "Mozilla/5.0" -o rules.zst "https://wii.leseratte10.de/devkitPro/devkitA64/devkita64-rules/devkita64-rules-1.1.1-1-any.pkg.tar.zst"
tar -C / -xf rules.zst
ls /opt/devkitpro/devkitA64 | head -5
mkdir -p $SRC && cd $SRC
[ -d libnx ] || git clone -q https://github.com/switchbrew/libnx.git
cd libnx
git checkout -q v4.12.0 || git checkout -q "$(git tag | sort -V | tail -1)"
make -j8 >/dev/null
make install >/dev/null
# --- Rust nightly + bevy-horizon
export RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
if ! command -v /usr/local/cargo/bin/rustc >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain nightly-2026-10-04 --component rust-src,llvm-tools-preview --profile minimal
fi
mkdir -p $SRC && cd $SRC
[ -d bevy-horizon ] || git clone -q https://github.com/JohnUzoka/bevy-horizon.git && git -C bevy-horizon checkout -q 3eac779b232ee47cd0eb957676c64386ac7b0f1d
echo "toolchain ready; next: build-nvk.sh"
