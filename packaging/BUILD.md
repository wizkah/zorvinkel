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

# Builds on Windows

Prerequisites: Rust with Cargo (MSVC toolchain, `x86_64-pc-windows-msvc`), the Visual Studio Build Tools and internet access for Cargo dependencies. Optional: the Windows SDK, whose resource compiler `rc.exe` embeds the application icon into the client; without it the build only warns and the executable keeps the default icon.

## Client

```powershell
cargo build --release --locked -p zorvinkel-client --features gui,ble-client,security
```

Creates `target\release\zorvinkel-client.exe`. The real Bluetooth backend needs `ble-client`; without hardware, `--simulate-gui` runs the built-in simulation. Data lives in `%LOCALAPPDATA%\Zorvinkel`, the client identity in the Windows Credential Manager.

## Device service

The device service `zorvinkeld` and `zorvinkel-admin` are built in WSL 2 (Ubuntu) with the same Raspberry Pi cross-build as on macOS. The script only calls Homebrew when Zig or `pkg-config` is missing, so install them in Ubuntu once:

```sh
sudo apt update && sudo apt install -y build-essential pkg-config curl xz-utils binutils libdigest-sha-perl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
mkdir -p "$HOME/zig" && curl -fsSL https://ziglang.org/download/0.13.0/zig-linux-x86_64-0.13.0.tar.xz | tar -xJ -C "$HOME/zig" --strip-components=1
```

Then run the script from the Windows checkout (mounted at `/mnt/d/...`). A Windows checkout may have CRLF line endings, which `sh` rejects, so strip them and point `root` at the checkout:

```sh
. "$HOME/.cargo/env" && export PATH="$HOME/zig:$PATH" && cd /mnt/d/src/zorvinkel
tr -d '\r' < packaging/linux/build-pi-zero.sh | sed "s|^root=.*|root=$PWD|" | sh
```

The output lands in `target/arm-unknown-linux-gnueabihf/release/` and `target/aarch64-unknown-linux-gnu/release/` of the checkout, as described above. `install.sh` reads its unit and configuration files from its own `packaging/linux` folder, so the Pi needs both the binaries and `packaging/`. From the checkout (Git Bash or WSL; use `target/arm-unknown-linux-gnueabihf/release` for a 32-bit device), replacing `pi` with your device's address:

```sh
S=$(mktemp -d) && mkdir "$S/bin" && cp target/aarch64-unknown-linux-gnu/release/zorvinkeld target/aarch64-unknown-linux-gnu/release/zorvinkel-admin "$S/bin/" && cp -r packaging "$S/"
(cd "$S" && tar cf - .) | ssh user@pi 'rm -rf ~/zorvinkel-install && mkdir ~/zorvinkel-install && tar xf - -C ~/zorvinkel-install'
ssh user@pi 'sudo ~/zorvinkel-install/packaging/linux/install.sh ~/zorvinkel-install/bin'
```

Without the cross-build, `cargo run --locked -p zorvinkeld -- --simulate` runs the service in simulation on Windows, which is what CI checks.

## Checks

```powershell
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

# Showing services

The device service can show further services of the device (e.g. Raspotify) in the device tab. Setup and file format: [Services.md](Services.md).
