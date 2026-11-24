# Graphical setup with simulation

**Date:** 2026-11-12

## Start

In the project directory:

```sh
cargo run --locked -p zorvinkel-client --features gui -- --simulate-gui
```

The simulation shows a photo booth without a network connection. After "Connect", Wi-Fi and device Wi-Fi can be set up and restart options chosen. The demo Wi-Fi networks use the password `demo-wlan-2026`; any other password of valid length simulates a failed connection attempt.

In the simulation, the display "Reachable from this computer" stays unchecked regardless of a successful Wi-Fi login; on real devices it results from the IP reachability test (ADR-020). Saved Wi-Fi passwords are not available in the simulation; manual entry remains usable. All settings live only in memory. No real devices or networks are changed.

## Verification

macOS ARM64, Rust/Cargo 1.99.1, egui/eframe 0.33.3 with Glow. Native GUI built and started as a temporary test app bundle. Device list, Wi-Fi view and device Wi-Fi view were inspected visually. The revised design with embedded logo and wordmark was checked visually in the device selection, the Wi-Fi view and the switched-off device Wi-Fi view.

Five integration tests check selected device as a prerequisite, correctable wrong password, separate IP reachability, state after reconnection, restart option, current AP credentials and manual entry after an unavailable password takeover. Two GUI unit tests check the abort before job execution and the Wi-Fi QR encoding of reserved characters. The entire workspace passes 67 tests with all features.

The background channel is bounded; the simulated delay takes place outside the UI thread. A job cancelled before execution reads the current state without executing the requested change.

The design change passes `cargo build --offline --locked -p zorvinkel-client --features gui` and the two existing GUI unit tests on macOS ARM64.

The native application icon uses the embedded bird logo on a light blue-grey surface, with rounded corners and a transparent margin on macOS. macOS build and Clippy with `-D warnings` passed. The Dock display could not be confirmed visually because of a timeout of the UI query.

The transparent macOS title bar was checked visually in the started window: continuous light header area, hidden window title and free native window buttons. Build and Clippy with `-D warnings` passed. Moving, full screen and minimising were not checked manually.

The SVG progress indicator passes the four GUI tests and Clippy with `-D warnings` on macOS ARM64. The animation test renders 164 frames of a wing-beat cycle and checks visible, changing image content. An additional test checks the visible ground movement at 96 × 64 pixels during a gliding phase. A rendered image of this size was checked visually; the integration in the running window was not checked visually.

## Action-related progress

Automatic status queries change neither the loading indicator nor the success message and block no input. User jobs during a status query are executed afterwards. The progress appears centred over the entire app window. Indicator and welcome image use the same display size of 210 × 122 pixels; the action text is below. There is no cancel button; the page layout stays unchanged.

On macOS ARM64, ten GUI tests pass, including silent status query, waiting user action, stable row height, centred and bounded overlay position and new connections after a previously cancelled job. Build and Clippy for the program code with `-D warnings` pass. Clippy including tests passes with the pre-existing lint `chunks_exact_to_as_chunks` in the animation tests excluded. App package and signature check pass.

In the updated app package, the overlay was checked visually while connecting to the Pi: animation at welcome size, centred position over the entire window, action text, no cancel button and no shifting of the page content. After the failed Bluetooth service was restored, the secure GUI connection works. Two consecutive secured connections with the same client registration additionally pass in the native command-line test. Automatic status queries stay without a loading indicator. No Wi-Fi connection changed.

## Device names and animation display

The native client shows the hostname without a permanent identifier. Different registered identities with the same stored hostname get a distinguishable key prefix, even with an unreachable second device. Automated tests check single devices, the same identity under several handles, different identities, prefix collisions, reopening the metadata and unchanged trust bindings with damaged name data. Two real devices with the same name were not checked.

The animation rasterises four image layers once and then moves prepared textures using graphics acceleration. The UI thread parses and rasterises no SVG images during the running animation. While the indicator is visible, the display requests further frames; native VSync determines the pace. A regression test checks reused textures and undelayed frame requests after initialisation. No additional animation timer.

128 workspace tests with all features passed; a native keychain test stays hidden. Clippy passes with the already documented exception for existing animation test lints. After the adjustment of the frame planning, the ten GUI tests pass again. The app is packaged as `target/Zorvinkel.app`; the suffix in the previous file name is dropped. In the native interface, `test-pi` is visible without an identifier and with a secure device connection. The prepared animation layers were checked visually in the central overlay; a measurement of the display frame rate actually achieved is outstanding.

## Design tokens (ADR-036)

**Date:** 2026-11-24. Windows 11, debug build, `--simulate-gui` with three devices, window about 960 × 660 px.

Before the change, the simulation was inspected visually: grey secondary buttons outside the palette, an empty box instead of a tick in the function list, a status column cut off at the bottom and right, fields lower than buttons. After the change, the device list, all pages of a device tab (Wi-Fi in both modes with a selected network, System, both service groups, Device with the shutdown confirmation, Terminal login) and the status column were checked visually in English and German. Ticks, wrapping and scrolling of the status column, field heights, the segmented control, the page tabs including hover, the network list and the red confirmation appear as intended; German texts fit without overflow.

`cargo fmt --check`, Clippy for `zorvinkel-client` with `gui`, `security` and `ble-client` and `-D warnings`, and 76 client tests pass.

Not checked: macOS and Linux, high-DPI scaling other than this display, the running terminal, error messages and action progress inside a device tab, a real device, contrast by measurement.

## Limits

No complete manual usability check, no QR scan with an independent decoder and no user acceptance. Windows/Linux GUI, minimum operating-system versions, accessibility and app packaging are unchecked. This experiment checks no real device connection, network configuration or credential takeover. The inputs simulate password-protected personal Wi-Fi; other security types are not checked.

IP reachability test (ADR-020), macOS ARM64: seven integration tests (`ip_reachability`) check, over loopback TCP, confirmed reachability, one working address among several, unregistered client, wrong device identifier, closed port and missing address as well as the restriction to `Authenticate` and the length check of the records. The entire workspace passes with `security` and `ble-client`, Clippy without warnings. The Linux service passes `cargo check` for aarch64 with the features `ble-server` and `security` (dummy for `dbus-1.pc`, no linking). The ARMv6 cross-build with sysroot succeeded; the service runs on the Pi Zero W (Raspbian 10) after `install.sh`, reports "BLE ready" and listens on port 47470, which is reachable from the Mac over TCP. Not checked: the display in the GUI against the Pi, client isolation, device Wi-Fi and the presentation in the running GUI.

Configured services (ADR-021), macOS ARM64: the entire workspace passes with `security`, `ble-server` and `gui`, Clippy without new warnings. Checked are codec and limits of the `Services` message (protocol test), permission, capability bit and error cases in the service, the configuration parser with invalid files, the state mapping of `systemctl show`, the version extraction, tab formation per group and fallback of vanished groups in the GUI as well as the choice of the address for the web link. The Linux service passes `cargo check` for aarch64 (dummy for `dbus-1.pc`, no linking); `packaging/linux/build-pi-zero.sh` builds the service for ARMv6 and aarch64 with `toml`/`serde` successfully. On the Pi Zero W (Raspbian 10) the service is updated with `install.sh` and starts ("BLE ready"); with `raspotify.toml`, `systemctl show raspotify.service` (active) and `librespot --version` ("0.3.1" as the second word) deliver the expected values under the service user. Not checked: the display in the GUI against the Pi over BLE, opening in the system browser, QR scan and the appearance of the tabs in the running GUI.
