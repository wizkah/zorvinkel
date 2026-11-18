#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
for tool in cargo rustup curl ar tar shasum; do
    command -v "$tool" >/dev/null || { echo "Fehlendes Werkzeug: $tool" >&2; exit 1; }
done
if ! command -v zig >/dev/null || ! command -v pkg-config >/dev/null; then
    command -v brew >/dev/null || { echo 'Homebrew für Zig und pkg-config erforderlich.' >&2; exit 1; }
    if ! command -v zig >/dev/null; then brew install zig; fi
    if ! command -v pkg-config >/dev/null; then brew install pkg-config; fi
fi
if ! cargo zigbuild --help >/dev/null 2>&1; then
    cargo install cargo-zigbuild --version 0.24.1 --locked
fi
rustup target add arm-unknown-linux-gnueabihf aarch64-unknown-linux-gnu
work=$(mktemp -d "${TMPDIR:-/tmp}/zorvinkel-sysroot.XXXXXX")
trap 'rm -rf "$work"' EXIT HUP INT TERM
mkdir -p "$work/package"
fetch() {
    curl -fsSL "$1" -o "$work/package/package.deb"
    printf '%s  %s\n' "$2" "$work/package/package.deb" | shasum -a 256 -c -
    (cd "$work/package" && ar -x package.deb && tar -xJf data.tar.xz -C "$3")
}
build() {
    target=$1
    arch=$2
    library=$3
    sysroot="$work/$target"
    mkdir -p "$sysroot"
    fetch "$4" "$5" "$sysroot"
    fetch "$6" "$7" "$sysroot"
    lib="$sysroot/usr/lib/$arch"
    rm -f "$lib/libdbus-1.so"
    ln -s "../../../lib/$arch/$library" "$lib/libdbus-1.so"
    PKG_CONFIG_ALLOW_CROSS=1 \
    PKG_CONFIG_SYSROOT_DIR="$sysroot" \
    PKG_CONFIG_LIBDIR="$lib/pkgconfig" \
    cargo zigbuild --release --locked -p zorvinkeld -p zorvinkel-admin \
        --features zorvinkeld/ble-server,zorvinkeld/security,zorvinkeld/network-manager \
        --target "$target.2.28"
}
cd "$root"
build arm-unknown-linux-gnueabihf arm-linux-gnueabihf libdbus-1.so.3.19.17 \
    'https://legacy.raspbian.org/raspbian/pool/main/d/dbus/libdbus-1-dev_1.12.28-0+deb10u1_armhf.deb' \
    31dfc04a99cc983faf0586fc30242cd6a5192c392ddf61588138f4a7fa8fa6d7 \
    'https://legacy.raspbian.org/raspbian/pool/main/d/dbus/libdbus-1-3_1.12.28-0+deb10u1_armhf.deb' \
    2aee8c64347c3d15db564c343996dfd28e949dc21f7ba5656f5ff36be885b4fb
build aarch64-unknown-linux-gnu aarch64-linux-gnu libdbus-1.so.3.19.13 \
    'https://archive.debian.org/debian/pool/main/d/dbus/libdbus-1-dev_1.12.20-0+deb10u1_arm64.deb' \
    93205e87e8d534f37e4fb4bb02458f2a1e86aeab8256e35cdd7462a2b8fa5a95 \
    'https://archive.debian.org/debian/pool/main/d/dbus/libdbus-1-3_1.12.20-0+deb10u1_arm64.deb' \
    8d1c11bd21a668894ac9b9a392c10521b77b3ab57b51ec14ca2db7f4f5a45de8
