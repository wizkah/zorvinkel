# Zorvinkel – Feasibility

**Date:** 2026-11-10  
**Scope:** Local Rust experiments and review of official documentation  
**Test environment:** macOS / Apple Silicon, Rust 1.99.1, target `aarch64-apple-darwin`

## Result

The local check shows no fundamental obstacle to the Rust approach. Noise communication is proven in practice; the macOS BLE client can be built. The full device architecture is not yet proven by this. No Raspberry Pi was available for this check.

## Checked in practice

The isolated experiment lives under `feasibility/noise-probe`. It is not product code and defines no final security profile. Dependencies are pinned by `Cargo.lock`.

| Experiment | Result |
|---|---|
| Noise XX with `snow` 0.10.0 | Handshake and bidirectional encryption successful |
| Identity keys | Both peer keys provable; deviation from an expected device key detectable |
| Tampered message | Decryption rejected |
| Repeated message of the same session | Rejected |
| Old message in a fresh session with the same identities | Rejected |
| Noise over a real local TCP socket | Handshake and encrypted request/response successful |
| Handshake size without payload | 32, 96 and 64 bytes with `Noise_XX_25519_ChaChaPoly_SHA256` |
| BLE adapter example with `btleplug` 0.13.5 | Compiled and linked for macOS; not executed |

**Tests:** 7 passed. The TCP test needs local sockets to be allowed; it binds only to `127.0.0.1` with a dynamic port. The sandbox initially denied this access; outside this restriction the test passed.

**Code check:** Clippy for all targets of the local package and all features passed without warnings.

The key proof is not a secure assignment at first contact. Likewise, the negative tests are not a complete security proof. The [Snow project description](https://github.com/mcginty/snow) explicitly names the absence of a formal audit; the choice of library remains to be reviewed.

### Repeating

In the project directory:

```sh
cargo test --locked --manifest-path feasibility/noise-probe/Cargo.toml -- --nocapture
cargo build --locked --manifest-path feasibility/noise-probe/Cargo.toml --features ble --example ble_adapter
cargo clippy --locked --manifest-path feasibility/noise-probe/Cargo.toml --all-targets --all-features -- -D warnings
```

When run manually, the BLE example checks only the adapter status. It starts no scan and sets up no device connection. The pure build checks no Bluetooth permission or radio function.

## Documented technical basis

| Area | Basis and limit |
|---|---|
| BLE client | [btleplug](https://github.com/deviceplug/btleplug) supports the central role on macOS, Windows and Linux. A different adapter is needed for the Linux GATT server. macOS needs the appropriate Bluetooth permission and, for the app bundle, `NSBluetoothAlwaysUsageDescription`. |
| BLE device service | [BlueR](https://github.com/bluez/bluer) offers local GATT services and advertising over BlueZ. Linux, D-Bus and the actual radio hardware were not tested here. |
| Network management | [Raspberry Pi OS](https://www.raspberrypi.com/documentation/computers/configuration.html) has used NetworkManager by default since Bookworm. Other network configurations need separate support. |
| Rollback | The [NetworkManager D-Bus API](https://networkmanager.pages.freedesktop.org/NetworkManager/NetworkManager/gdbus-org.freedesktop.NetworkManager.html) offers checkpoints with automatic time-controlled rollback. Behaviour on restart and with own storage changes remains to be solved separately. |
| Rust interface | [Slint](https://slint.dev/faqs) supports Rust as well as macOS, Windows and Linux. Candidate for a GUI experiment; neither the toolkit choice nor packaging is decided by this check. |

These statements prove available interfaces, not a tested overall combination.

## Not yet proven

| Check | Required evidence |
|---|---|
| BLE end to end | macOS client against Raspberry Pi GATT server: discovery, write, notifications, abort and reconnect |
| BLE fragmentation | Transfer messages above the available ATT payload completely and bounded; check small packets, missing or duplicate fragments, ordering and timeout |
| Registration and storage | Concurrent first contacts, persistent commit, lost response, restart and damaged storage |
| Wi-Fi and device AP | Wrong password, AP switching and rollback with BLE still operable; then cold start |
| BLE → IP | Find the endpoint, verify the same identities, bind sessions and keep using BLE on failure |
| Permissions and load | Service without blanket root rights; bounded resources alongside the actual device application |
| Platform integration | Windows/Linux builds, GUI packaging, operating-system key stores and optional Wi-Fi password takeover |

The local Bluetooth system report returned no controller. No statement about the physical hardware or its fundamental suitability can be derived from it.

## Next experiment with target hardware

A Raspberry Pi with BLE/Wi-Fi and NetworkManager and a macOS client are sufficient for the first end-to-end experiment:

1. Publish a small GATT service with write and notifications.
2. Transfer Noise messages over bounded BLE frames; check reconnect and identity deviation.
3. Initially connect only network status and Wi-Fi scan.
4. On a test device, switch Wi-Fi and AP, provoke errors and observe BLE reachability.
5. Only then add persistent registration, restart behaviour and IP channel binding.

These experiments prove no trustworthy assignment at first contact and no secure advance distribution of public keys.
