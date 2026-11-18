# BLE verification

**Date:** 2026-11-12

## Environment and scope

macOS ARM64, rustc and Cargo 1.99.1. Radio test against Raspberry Pi Zero W Rev 1.1, ARMv6, Raspbian 10, kernel 5.10.103+, glibc 2.28 and BlueZ 5.50. Dependencies are in the workspace lock file. The current search check was made at a distance of 2–3 m according to the user; the distance was not measured.

| Check | Result |
|---|---|
| Frame integration tests | Nine passed |
| Probe simulation tests | Four passed |
| Native macOS client including program entry point | Built and linked |
| Entire workspace without BLE features | Tests passed |
| Linux GATT test service | Built for ARMv6 and started on the Pi |
| Advertising and macOS search | Service `719e40a0-4a4b-4d75-9c08-a9cbb88f4100` and name `ZV-P3-967C` detected |
| Write / notifications | Two regular CLI runs with two connections each passed; per connection 256 bytes checked in both directions |
| BlueZ parameter regressions | Four tests on the Pi passed: old and new MTU options, missing MTU, invalid types/values and missing device assignment |
| Reconnect | Two consecutive connections in both regular CLI runs passed |
| Event-based search, original advertising interval | Six of six search runs successful, including a search after a service restart |
| Four individual search durations | Around 1.0, 2.8, 3.4 and 9.5 seconds |
| Control test with 100 ms advertising interval | Unchanged old search succeeded five of five times; two BLE connections passed |
| Search with the test service stopped | Ended without a hit; no old device entry reported |
| Ending with Ctrl-C | Process ended, advertising instances back to zero and test service removed from the UUID list |
| Linux/Windows builds with BLE features | CI set up, not run |

Checked are fragmentation at frame sizes of 20, 64 and 512 bytes, boundary lengths, encoding, faulty headers, invalid versions and types, missing, duplicate and swapped fragments, changing identifiers and lengths, absolute timeouts and backwards-running test time. Simulations check both transmission directions, discarded partial messages and new sessions after an abort. Payloads other than the fixed test pattern are rejected without any management action.

## Repeating the local check

```sh
cargo fmt --all -- --check
cargo build --workspace --all-targets --all-features --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

## Repeating the radio test

The Linux target device needs BlueZ and the D-Bus runtime library. D-Bus development files and `pkg-config` are needed only for the build. The adapter must be switched on; the test does not switch it on itself. On macOS, Bluetooth permission and, later, suitable app packaging are required.

Precompiled program on the device:

```sh
sudo /path/to/zorvinkeld --ble-probe
```

Client: search, then select the printed handle. The second call checks two separate connections.

```sh
cargo run --locked -p zorvinkel-client --features ble-client -- --ble-probe
cargo run --locked -p zorvinkel-client --features ble-client -- --ble-probe '<device-handle>'
```

Ending with Ctrl-C removes service and advertising. A subsequent service start with search and two connections was checked.

## Hardware findings

The controller is switched on. The user `pi` may not query BlueZ over D-Bus; the bounded test service therefore ran with `sudo`. This is not an acceptance of the later service permissions. The earlier snapshot after five seconds repeatedly stayed without a hit, even at about 30 cm distance. With event-based search, six search runs at the original advertising interval were completed successfully.

The D-Bus capture confirms `MTU` when acquiring the notification and a missing MTU for `WriteValue`. BlueR is used with the local parameter correction. The original acknowledged write path returned an ATT error after five seconds despite a successful D-Bus response. With the additionally offered write property `write-without-response` and client writes that are still acknowledged, the exchange works. Sources: [BlueZ 5.50](https://github.com/bluez/bluez/blob/5.50/src/gatt-database.c), [BlueR 0.17.4](https://docs.rs/bluer/0.17.4/src/bluer/gatt/local.rs.html).

The controller announces the correct service at an interval of 1.28 seconds. A 20-second observation reported only two matching macOS events and finally −83 dBm RSSI. Another observation delivered hints at around 4.6, 8.4, 13.6 and 18.7 seconds and finally −87 dBm. The event counts are not a measurement of all radio packets sent or received.

A temporary shortening to 100 ms was confirmed on the controller. The old five-second search runs found the Pi five of five times with it. Afterwards the original kernel values `2048/2048` were restored and checked. This diagnosis uses [kernel debug interfaces](https://github.com/torvalds/linux/blob/v5.10/net/bluetooth/hci_debugfs.c); the client does not change them.

The adapted search collects events instead of a single snapshot. At the original advertising interval, six search runs were successful. Two complete regular CLI runs each contained a search and two separate connections; one of them followed a service restart. For each connection, 256 bytes were transferred in both directions in 32 frames of at most 20 bytes and the content was checked. After the test service was stopped, the search ended without a hit.

During the diagnosis, the Bluetooth service was restarted and the controller was switched off and on again over D-Bus. The system stayed on BlueZ 5.50. No change to network management took place.

A snapshot of the waiting debug service showed about 6 MiB RSS and 0.8–1.0 percent averaged CPU usage. This is not a load measurement and no resource budget.

## ARMv6 build

The Mac uses `cargo-zigbuild` 0.24.1, Zig 0.13.0, `pkg-config` and the Rust target `arm-unknown-linux-gnueabihf`. Headers, `dbus-1.pc` and the actual D-Bus library were copied from the Pi into a temporary sysroot; absolute library symlinks must point to local targets there.

```sh
PKG_CONFIG_ALLOW_CROSS=1 \
PKG_CONFIG_SYSROOT_DIR="$PI_SYSROOT" \
PKG_CONFIG_LIBDIR="$PI_SYSROOT/usr/lib/arm-linux-gnueabihf/pkgconfig" \
cargo zigbuild --locked -p zorvinkeld --features ble-server \
  --target arm-unknown-linux-gnueabihf.2.28
```

The ELF program was copied to `/tmp/zorvinkel-mvp-20261111/`. `--simulate` ran successfully on the Pi. Rust is required only on the build machine. The temporary programs and diagnostic recordings were removed from the Pi after the check. The operating system and packages were not updated; network management stayed unchanged.

## Failure and recovery

On 2026-11-12 at 10:14 the test Pi reports a memory error for bluetoothd (`free(): invalid next size (fast)`) and abort with SIGABRT. The Zorvinkel service stays active; connection attempts from the Mac end with "Device disconnected". A link to a GUI abort action is not proven.

After restarting Bluetooth and then Zorvinkel, two consecutive secured status connections with the existing client registration and one secure connection through the GUI succeed. No device identity or network configuration was set up again. The cause of the crash remains open.

## Automatic restart

On the Zero W with systemd 241, a Bluetooth drop-in and the dependent Zorvinkel unit are installed. Both use a three-second restart pause and at most five starts in 60 seconds. Bluetooth pulls Zorvinkel in at start; Zorvinkel is ended on a Bluetooth failure and registers BLE again after the restart.

The unit check on the Pi and the shell syntax check of the installer pass. A targeted SIGKILL of the Bluetooth main process leads, without a manual start, to new main processes of both services within 5.31 seconds. The new Zorvinkel reports BLE readiness; its administration socket returns the same device identity. The contents of the Wi-Fi, DHCP and interface configuration stay identical. Two subsequent native status connections from the Mac with the same client registration succeed.

The complete installer was not run again for this test; its contained unit files were installed directly. Start limit, cold start with these rules, hanging processes and other platforms are not practically accepted. The underlying BlueZ memory violation remains unexplained.

## Device name

On start, the device service reads the active Linux hostname from `/proc/sys/kernel/hostname` and uses it in the BLE display name with the previous device identifier. On the test Pi, the active and the recorded hostname agree with `test-pi`. After the update and a service restart, `test-pi-967C` is reported as BLE readiness and was received in a fresh search with the signed macOS app program. Device identity as well as hostname, Wi-Fi, DHCP and interface configuration stay unchanged.

ARMv6 release build, Clippy for the device service with BLE and security without warnings, formatting and diff check pass. A changed hostname is taken over at the next service start.

## Limits

The six successful search runs are a limited sample for one device/client combination. Other distances, shielding and radio environments were not accepted with the adapted search. The capture showed an ATT MTU of 185; the test still uses at most 20 bytes per frame. No acceptance with a forced minimum MTU, radio loss, competing clients or load. Newer BlueZ parameters were checked through regression tests, not against a second real daemon. Independent security, productive permissions and BLE during network switches are unchecked. The own resource limits do not replace a measurement of the Bluetooth and library buffers. P3 is not accepted.
