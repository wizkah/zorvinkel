# Device management, IP channel and sessions

**Date:** 2026-11-16

Concerns ADR-016 (shutdown/restart), ADR-019 (system information), ADR-020 (IP reachability), ADR-021 (service list), ADR-022 (service actions), ADR-023 (IP channel, mDNS search) and ADR-024 (session renewal, fallback, correlation, connection path).

## Environment

macOS ARM64, rustc/Cargo 1.99.1. `cargo test --offline --locked --workspace --all-features`: 262 tests passed, 2 ignored (real mDNS run and native keychain test). `cargo fmt --check` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` pass. Network stacks, systemd and radio are simulated in these tests.

## Checked by tests

- Protocol: messages for `Power`, `SystemInfo`, `Services`, `ServiceAction` and `Correlation*` are typed, bounded and separate from other capability bits; wrong lengths are rejected.
- Service over the encrypted channel: each of these operations requires registration and is offered only if the service supports it; service actions stay within the configuration.
- Shutdown/restart: a separate request file per action, further requests rejected until execution, new attempt after an error, capability only with enabled path units; GUI hint only after a successful action.
- Service list and actions: parser (default values, invalid files, missing directory), state from `systemctl show`, version extraction, queue only for permitted actions with a limit, helper checks again against the configuration and removes every request even on rejection.
- IP reachability and IP channel: loopback integration tests (confirmed reachability, wrong device identifier, unregistered client, closed port); a real mDNS run on macOS (ignored test, executed beforehand).
- Sessions: renewal shortly before expiry except when the option is deselected; correlation proof redeemable only over the IP endpoint, exactly once and time-limited.
- GUI: service tabs (groups, address choice for the web link), simulation with up to 100 devices.

## On the Raspberry Pi Zero W (Raspbian 10)

- Service with all named functions installed with `install.sh` and started; port 47470 reachable from the Mac over TCP; service list: `systemctl show` and version command deliver the expected values under the service user (example `raspotify`).
- Restart of the device over BLE and GUI: checked successfully by the user.
- Display of the connection path (Bluetooth/network): confirmed by the user (2026-11-15).

## Not checked

- Shutdown on the Pi.
- System information on the Pi (operation, `df` synchronous in the service, under-voltage display without the `video` group).
- Handshake and GUI display of IP reachability against the Pi; real networks with client isolation, device Wi-Fi, firewall.
- mDNS announcement on the Pi (Avahi coexistence, hostname collision) and first registration over the network.
- Display of the service tabs in the GUI against the Pi, service actions on the Pi, opening the web interface in the browser, QR scan.
- Session renewal after 10 minutes, fallback IP→BLE on a real Wi-Fi switch and correlation proof over BLE and IP on the Pi.
- Windows and Linux client; load limits of the IP endpoint (15 s handshake, 60 s idle) as a measurement.
