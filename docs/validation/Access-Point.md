# Device Wi-Fi

**Date:** 2026-11-16

## Environment

macOS ARM64, Rust 1.99.1, `cargo test --offline --locked --workspace --all-features`. All tests pass. No Raspberry Pi, no Linux build of the service with `ble-server`; `zorvinkel-adapters` with `security` was checked for `aarch64-unknown-linux-gnu` with `cargo check`.

## Checked

- Protocol: encoding, limits, ASCII password rules, redaction in `Debug`, capability bit separate from all others.
- Service over the encrypted channel: no change without registration; read, write, switch on/off; password never in plain text in the channel; without a helper no capability and `Unsupported`.
- Suggestion: name formation from the hostname, 63 characters without distribution bias, no suggestion without a randomness source.
- wpa_supplicant adapter with a simulated control channel: AP profile (`mode=2`, CCMP, channel 6), switching on, abort on helper failure, status detection, switching off only for own profiles (also after a service restart, foreign profiles untouched), rollback.
- Helper integration (request files) with a simulated helper; credentials file private, atomic, damaged data rejected; Wi-Fi controller: permission, exclusion, deduplication.
- GUI: page renders with suggestion, invalid input, active AP (QR) and without data.

## Addendum 2026-11-16 (ADR-025, ADR-026)

Environment as above (macOS, simulated network stacks). Additionally checked: credentials in protected storage (permissions 0700/0600, rejection of damaged or too open data, takeover and removal of the old file, one-time generation, no generation without a randomness source or over damaged data); rollback logic for the volatile device Wi-Fi, precedence of the default over "keep", restoration of the default (Wi-Fi by SSID, device Wi-Fi from credentials) at start and after a rollback, rejection without an active connection and for foreign access points; both stacks find saved profiles by SSID; operations and capability bit on the encrypted channel. Not checked on the Pi and not against a real NetworkManager.

## Raspberry Pi 4 Model B (2026-11-18)

Raspberry Pi OS (Debian 12 Bookworm), kernel 6.12.47+rpt-rpi-v8, aarch64, systemd 252, BlueZ 5.66, NetworkManager 1.42.4. Service and helper installed with `install.sh` from the ARM64 cross-build; the service runs. The user tested the Wi-Fi setting and device Wi-Fi (AP) manually with the macOS GUI and reported success. Scope and individual observations (DHCP lease of a phone, QR scan, return to the Wi-Fi, restart with "keep", rollback) are not recorded.

## Raspberry Pi Zero W (2026-11-16)

Updated service installed with `install.sh` (ARMv6 cross-build). The service runs; on start `/var/lib/zorvinkel/access-point-credentials/` was created (directory 0700, `lock` and `state` 0600, `state` 110 bytes), and the old file `access-point` no longer exists. Whether it existed before and was taken over is not documented; its content was not read. Default, "keep" after a restart and the rollback of the volatile device Wi-Fi remain unchecked on the Pi.

## Raspberry Pi Zero W (2026-11-15)

Raspbian 10, wpa_supplicant 2.8-devel, dhcpcd, dnsmasq 2.80 (`dnsmasq-base` installed afterwards). The chip reports mode `AP`. Service and helper installed with `install.sh`; the helper was checked on a dummy interface (address, dnsmasq, `ap-ready`, clean-up). The user tested the full flow with the macOS GUI (AP on/off over BLE) manually and reported success; scope and individual observations (DHCP lease of a phone, QR scan, return to the Wi-Fi, restart with "keep") are not recorded.

## Not checked

First start with generation and takeover of the old file on the Pi, default and volatile rollback of the device Wi-Fi on the Pi, individual evidence for the manual Pi test (see above), restart with "keep", address collisions, Bluetooth stability when switching, individual evidence for the manual Pi 4 test, Windows/Linux GUI.
