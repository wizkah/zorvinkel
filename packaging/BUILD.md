# Builds on macOS

Prerequisites: Rust with Cargo, Xcode Command Line Tools and internet access for Cargo dependencies.

## Client

```sh
packaging/macos/build.sh
```

Creates `target/Zorvinkel.app` with a local ad-hoc signature. An optional argument sets a different output path; the path must not exist yet.

## Raspberry Pi: 32 and 64 bit

The script installs missing Zig and pkg-config with Homebrew, cargo-zigbuild with Cargo and the Rust targets with rustup. Homebrew and rustup must be present. Tools checked: Zig 0.13.0 and cargo-zigbuild 0.24.1.

```sh
packaging/linux/build-pi-zero.sh
```

The script downloads D-Bus packages for ARMv6 from Raspbian 10 and for ARM64 from Debian 10, verifies their SHA-256 and unpacks them temporarily. It builds `zorvinkeld` and `zorvinkel-admin` for each:

- 32 bit: `target/arm-unknown-linux-gnueabihf/release/`
- 64 bit: `target/aarch64-unknown-linux-gnu/release/`

The target is glibc 2.28 or newer. The device needs D-Bus and BlueZ. The device's own Wi-Fi additionally needs `dnsmasq-base`; without that package `install.sh` does not enable the helper and the function stays hidden. ARM64 is checked on a Raspberry Pi 4 (Debian 12, NetworkManager) for Wi-Fi and the device's own Wi-Fi.

## Showing services

The device service can show further services of the device (e.g. Raspotify) in the device tab. Setup and file format: [Services.md](Services.md).
