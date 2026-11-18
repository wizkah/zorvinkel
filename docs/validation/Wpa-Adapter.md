# Volatile Wi-Fi adapter

**Date:** 2026-11-12

## Scope

macOS ARM64, Rust 1.99.1. Linux adapter checked with a simulated wpa_supplicant control channel and a real local Unix datagram socket.

- Eleven adapter tests passed: temporary profiles without a save command, input check, profile clean-up after an error, connection time limit, return to the previous connection and activation states, reused profile identifiers, ownership check, scan decoding, AP rejection and socket clean-up.
- Workspace tests with all features passed; the explicitly hidden native keychain test was not executed.
- Clippy for the adapter with all targets and features passed without warnings. Formatting and diff check passed.
- Adapter, tests and manual check program cross-built and linked for ARMv6/glibc 2.28.

## Test Pi on 2026-11-11

Raspberry Pi Zero W, ARMv6, Raspbian 10, wpa_supplicant 2.7 and dhcpcd 8.1.2. Eleven cross-compiled adapter tests passed. The native check program reads status and scan cache and creates a disabled test profile, checks it and removes it. Both calls pass as the user `zorvinkel` with group `netdev`.

A before/after check confirms identical contents of `/etc/wpa_supplicant/wpa_supplicant.conf`, `/etc/dhcpcd.conf` and `/etc/network/interfaces`, an unchanged Wi-Fi connection and profile list and the same PID of the running device service. Its autostart is enabled. No Wi-Fi switch and no device restart in this experiment.

## BLE/GUI integration on 2026-11-12

On macOS ARM64, 119 workspace tests pass with all features; a native keychain test stays explicitly hidden. Additional checks cover the bounded Wi-Fi message contract, encryption and authorisation, capability negotiation, ownership and deduplication of waiting jobs as well as fresh scan completion events. Formatting and diff check pass.

Clippy passes with all targets and features with the lint `chunks_exact_to_as_chunks` excluded. The strict full run reports three pre-existing findings in the animation tests; these were not changed. Desktop client built with all features; macOS app package `target/Zorvinkel.app` created, signature and plist checked successfully. Updated device service built optimised and linked for ARMv6/glibc 2.28.

The updated device service was installed and restarted on the Zero W after approval. Device identity and contents of the network configuration files stay unchanged. The updated macOS app finds the Pi, connects securely, shows the actual Wi-Fi status and offers Wi-Fi setup. A fresh scan triggered through the GUI returns four networks. The view with Wi-Fi selection, name and password input and a hint about volatile changes was checked visually. No Wi-Fi switch triggered.

## Repeating

```sh
cargo test --offline --locked --workspace --all-features
cargo clippy --offline --locked -p zorvinkel-adapters --all-targets --all-features -- -D warnings
```

On Linux with an existing wpa_supplicant, dhcpcd and access to `/run/wpa_supplicant/wlan0`:

```sh
cargo run --locked -p zorvinkel-adapters --example wpa-runtime
cargo run --locked -p zorvinkel-adapters --example wpa-runtime -- --probe-profile
```

The second call creates a disabled test profile, checks it and removes it. It selects no Wi-Fi. Both calls read the existing scan cache without triggering a new scan. Errors can require a manual clean-up of the temporary profile.

## Adapter selection

On the test Pi (no NetworkManager, running dhcpcd, control socket `wlan0`) the updated service was installed. It starts, reports BLE readiness `test-pi-967C` and selects the fallback. Identity and name are preserved. The selection path with a running NetworkManager as well as a missing control socket or dhcpcd is covered only by unit tests. Wi-Fi search and capability report after the update were not checked again through the GUI.

## Opt-in persistence

The switch `wifi-persistence` is off by default and can be changed only locally through `zorvinkel-admin`. Checked automatically: reactivation of foreign profiles before `SAVE_CONFIG`, abort on a foreign or changed connection. On the test Pi (`update_config=1`), `status`, `on`, `off` work; invalid values are rejected. A real `SAVE_CONFIG` after a BLE/GUI connection was not executed, the Pi configuration stayed unchanged. If saving fails, the connection stays and the client does not learn of it; the profile stays volatile.

## Wi-Fi switch through the GUI on 2026-11-14

Test Pi as above, client macOS. The "keep/original" choice (BLE operation `WifiPersistence`) is connected in the device service and in the GUI; switching to a second hotspot through the GUI succeeded, a wrong password led to the return to the previous network. Not executed: a real `SAVE_CONFIG` through the GUI choice and a restart afterwards.

When switching to an iPhone hotspot the Bluetooth chip hung several times (`link tx timeout`, `bluetoothd` crashed), the GUI no longer connected although the Wi-Fi was up. The fallback after 60 s without client contact did not take effect, not even after a service restart; the Pi stayed in the hotspot until it was switched off. `wpa_cli reconfigure` restores the saved configuration on its own (before: six profiles, after: only the home network active), but itself triggered a Bluetooth hang again. A remaining edge case that is still open.

## Limits

Real Wi-Fi switching and wrong password are checked individually; dhcpcd behaviour after a link change and BLE reachability during changes are not reliable (see above). No AP/DHCP server integration, no IP reachability test and no restart configuration except the "keep/original" choice. The native BLE/GUI search is checked; connection change and error handling have so far been checked only automatically with simulated system access. Returning to the previous connection needs the running adapter instance. After a process abort, the operation journal is reconciled with the actual state; an automatic restoration of the previous Wi-Fi connection is thus not proven. Simultaneous external management programs and their save operations are not controlled.
