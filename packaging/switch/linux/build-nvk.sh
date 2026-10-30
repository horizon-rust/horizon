#!/usr/bin/env bash
# Builds NVK (Mesa's Vulkan driver, from mesa-switch + bevy-horizon's patches) and localizes it into
# $SRC/bevy-horizon/nvk/lib/libvulkan_bh.a. Run as root after setup-toolchain.sh; about 5 minutes.
set -eo pipefail
export SRC="${SRC:-/root/src}"
export DEBIAN_FRONTEND=noninteractive
export DEVKITPRO=/opt/devkitpro
export PATH=/usr/local/cargo/bin:/opt/devkitpro/devkitA64/bin:/opt/devkitpro/tools/bin:/usr/local/bin:/usr/bin:/bin
export RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
JOBS=${JOBS:-4}
P=/opt/devkitpro/portlibs/switch
mkdir -p $P/lib $P/include $P/bin /tmp/dl
CF="-O2 -march=armv8-a+crc+crypto -mtune=cortex-a57 -mtp=soft -fPIE -D__SWITCH__"

echo "== zstd"
if [ ! -f $P/lib/libzstd.a ]; then
  cd /tmp/dl
  curl -fsSL -o zstd.tgz https://github.com/facebook/zstd/releases/download/v1.5.7/zstd-1.5.7.tar.gz
  rm -rf zstd-1.5.7 && tar xzf zstd.tgz && cd zstd-1.5.7/lib
  make -j$JOBS libzstd.a CC=aarch64-none-elf-gcc AR=aarch64-none-elf-ar RANLIB=aarch64-none-elf-ranlib CFLAGS="$CF" ZSTD_LEGACY_SUPPORT=0 HAVE_PTHREAD=0 ZSTD_MULTITHREAD_SUPPORT=0 >/dev/null
  cp libzstd.a $P/lib/; cp zstd.h zdict.h zstd_errors.h $P/include/
  cat > $P/lib/pkgconfig_zstd.txt <<EOT
EOT
fi
echo "== expat"
if [ ! -f $P/lib/libexpat.a ]; then
  cd /tmp/dl
  curl -fsSL -o expat.txz https://github.com/libexpat/libexpat/releases/download/R_2_7_1/expat-2.7.1.tar.xz
  rm -rf expat-2.7.1 && tar xJf expat.txz && cd expat-2.7.1
  ./configure --host=aarch64-none-elf --disable-shared --enable-static --without-docbook --without-tests --without-examples --without-xmlwf --prefix=$P CC=aarch64-none-elf-gcc AR=aarch64-none-elf-ar RANLIB=aarch64-none-elf-ranlib CFLAGS="$CF" >/dev/null
  make -j$JOBS -C lib >/dev/null && make -C lib install >/dev/null
fi
ls $P/lib $P/include | head -30

echo "== rust target, bindgen, cbindgen"
rustup target add aarch64-unknown-linux-gnu
apt-get install -y --no-install-recommends bindgen cbindgen 2>&1 | tail -1 || true
command -v bindgen cbindgen || cargo install --locked bindgen-cli cbindgen

cd $SRC && [ -d mesa-switch ] || git clone -q https://github.com/danfromtico/mesa-switch.git
export DEVKITPRO=/opt/devkitpro
export PATH=/usr/local/libexec:/usr/local/cargo/bin:/opt/devkitpro/devkitA64/bin:/opt/devkitpro/tools/bin:/usr/local/bin:/usr/bin:/bin
export RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo
export LLVM_CONFIG=llvm-config-19
JOBS=${JOBS:-4}
P=/opt/devkitpro/portlibs/switch
BH=$SRC/bevy-horizon
cd $SRC/mesa-switch
git checkout -q . && git clean -fdq -e builddir-native -e builddir-switch
for p in $BH/patches/mesa-switch/*.patch; do echo "patch $(basename $p)"; git apply "$p"; done
MESA_COMMIT=$(git rev-parse HEAD)

mkdir -p /usr/local/libexec
cp bindgen-switch-wrapper.sh /usr/local/libexec/bindgen
cp rustc-switch-wrapper.sh /usr/local/libexec/rustc
cp bindgen-atomic-shim.h /usr/local/libexec/bindgen-atomic-shim.h
sed -i 's#\*sanity_check_for_rust.rs\*|\*sanitycheckrs.rs\*)#*sanity_check_for_rust.rs*|*sanitycheckrs.rs*|*/sanity.rs*)#' /usr/local/libexec/rustc
grep -n 'sanity' /usr/local/libexec/rustc
# bindgen (Debian) rejects Meson's "--rust-target 1.101.0-nightly": drop the option.
python3 - <<'PY'
p = "/usr/local/libexec/bindgen"
s = open(p).read()
tail = 'exec "$BINDGEN_BIN" "$@"'
assert tail in s
fix = """n=$#
i=0
while [ "$i" -lt "$n" ]; do
    arg="$1"; shift; i=$((i+1))
    if [ "$arg" = "--rust-target" ]; then shift; i=$((i+1)); continue; fi
    set -- "$@" "$arg"
done
"""
open(p, "w").write(s.replace(tail, fix + tail))
PY
chmod +x /usr/local/libexec/bindgen /usr/local/libexec/rustc
export MESA_SWITCH_RUSTC=/usr/local/cargo/bin/rustc
export MESA_SWITCH_BINDGEN=/usr/bin/bindgen
export MESA_SWITCH_RUST_TARGET=aarch64-unknown-linux-gnu

cp src/gallium/winsys/nouveau/drm/nouveau.h src/nouveau/headers/nv_device_info.h $P/include/
echo "void __mesa_switch_stub_lib(void) {}" > /tmp/stub_posix.c
aarch64-none-elf-gcc -c /tmp/stub_posix.c -o /tmp/stub_posix.o
for L in dl rt util; do
  [ -f $P/lib/lib$L.a ] || aarch64-none-elf-ar rcs $P/lib/lib$L.a /tmp/stub_posix.o
done

setup() { local dir=$1; shift
  if [ -f "$dir/meson-private/coredata.dat" ]; then meson setup "$dir" --reconfigure "$@"; else meson setup "$dir" "$@"; fi; }

echo "=== Native host tools (mesa_clc, vtn_bindgen2) ==="
setup builddir-native \
  -Dvulkan-drivers= -Dgallium-drivers= -Dshader-cache=enabled -Dplatforms= \
  -Dglx=disabled -Degl=disabled -Dopengl=false -Dgles1=disabled -Dgles2=disabled \
  -Dtools=[] -Dllvm=enabled -Dmesa-clc=enabled -Dprecomp-compiler=enabled \
  -Dinstall-mesa-clc=true
ninja -C builddir-native -j "$JOBS" src/compiler/clc/mesa_clc src/compiler/spirv/vtn_bindgen2

echo "=== NVK for Horizon ==="
export PATH="/usr/local/libexec:$SRC/mesa-switch/builddir-native/src/compiler/clc:$SRC/mesa-switch/builddir-native/src/compiler/spirv:$PATH"
printf "[binaries]\nrust = '%s'\n" "$MESA_SWITCH_RUSTC" > /usr/local/libexec/nvk-native.txt
setup builddir-switch \
  --cross-file switch_cross_file.txt --native-file /usr/local/libexec/nvk-native.txt \
  --default-library=static --buildtype=release -Doptimization=2 \
  -Db_lto=false -Db_ndebug=true \
  -Dvulkan-drivers=nouveau -Dgallium-drivers= -Dgallium-rusticl=false \
  -Dplatforms=switch -Dglx=disabled -Degl=disabled -Dopengl=false \
  -Dgles1=disabled -Dgles2=disabled -Dvideo-codecs= \
  -Dshader-cache=enabled -Dxmlconfig=enabled -Dexpat=enabled -Dtools=[] \
  -Dllvm=disabled -Dshared-glapi=disabled -Dshared-llvm=disabled \
  -Dmesa-clc=system -Dprecomp-compiler=system -Dcpp_rtti=false \
  -Dbuild-tests=false -Dnvk-build-id="$MESA_COMMIT"
ninja -C builddir-switch -j "$JOBS" src/nouveau/vulkan/libvulkan.a
aarch64-none-elf-ranlib builddir-switch/src/nouveau/vulkan/libvulkan.a

OUT=$BH/nvk
rm -rf $OUT; mkdir -p $OUT/lib $OUT/include
cp builddir-switch/src/nouveau/vulkan/libvulkan.a $OUT/lib/
cp -r include/vulkan include/vk_video $OUT/include/

BIN=/opt/devkitpro/devkitA64/bin/aarch64-none-elf
cd $SRC/bevy-horizon/nvk/lib
work=$(mktemp -d)
KEEP_RE='^(vk[A-Z]|nouveau_horizon_region_(alloc|free|stats)$|nouveau_horizon_tune$)'
$BIN-nm -g --defined-only libvulkan.a 2>/dev/null | awk -v re="$KEEP_RE" 'NF == 3 && $2 ~ /^[TDRBVW]$/ && $3 ~ re { print $3 }' | sort -u > "$work/keep.txt"
echo "$(grep -c '^vk' $work/keep.txt) vk* + $(grep -vc '^vk' $work/keep.txt) other API symbols kept"
grep -E 'vkGetInstanceProcAddr|nouveau_horizon' $work/keep.txt
$BIN-ld -r --force-group-allocation -o "$work/vulkan_bh.o" $(sed "s/^/-u /" "$work/keep.txt") libvulkan.a
$BIN-objcopy --keep-global-symbols="$work/keep.txt" "$work/vulkan_bh.o"
$BIN-nm -g --defined-only "$work/vulkan_bh.o" | awk -v re="$KEEP_RE" '$3 !~ re' > "$work/leftover.txt"
if [ -s "$work/leftover.txt" ]; then echo "leftover globals:"; head -20 "$work/leftover.txt"; exit 1; fi
if $BIN-readelf -g "$work/vulkan_bh.o" | grep -q "COMDAT group"; then echo "COMDAT groups survived"; exit 1; fi
$BIN-nm -u "$work/vulkan_bh.o" | awk '{ print $NF }' | sort -u > undefined_bh.txt
rm -f libvulkan_bh.a; $BIN-ar rcs libvulkan_bh.a "$work/vulkan_bh.o"
echo "undefined for final link: $(wc -l < undefined_bh.txt)"; ls -la
