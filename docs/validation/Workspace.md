# Rust workspace and state flows

**Date:** 2026-11-11

## Environment

macOS ARM64 (`aarch64-apple-darwin`), rustc 1.99.1, Cargo 1.99.1. Additionally Raspberry Pi Zero W, Raspbian 10, ARMv6 and kernel 5.10.103+: test programs precompiled on the Mac, without Rust on the Pi.

## Verification

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed |
| `cargo build --workspace --all-targets --all-features --locked` | Seven crates with optional macOS BLE client and GUI built |
| `cargo test --workspace --all-features --locked` | 93 tests passed (seven unit, 85 integration and one compile-fail test) |
| `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` | Passed |
| `cargo run --locked -p zorvinkel-client -- --simulate` | Device found, network status read |
| `cargo run --locked -p zorvinkeld -- --simulate` | Network status read |

Checked are search and status, concurrent first registrations in memory and through separate file-storage handles, repeated retrieval after a lost response, targeted grant and expiry, revocation and client loss. File tests check reopening, restrictive permissions, missing and damaged data, temporary write remnants and commit failures.

Setup tests check authorised key takeover before and after TOFU, atomic registration closing, device binding, concurrent first bindings and the rejection of unknown, blocked and deviating keys. Identity tests check reopening without key replacement, restrictive permissions, damaged and missing data and the internal restoration of the stored key material. A compile-fail test checks the missing public access to the private key field. Only synthetic key bytes were used.

Network tests check authorisation, deduplication, conflicts, rollback, unknown actual state, both restart options and errors before and after a system change. A simulated abort after the journal commit stays pending after reopening and is reconciled without repetition. Codec tests reject every truncation and single bit changes of the checked snapshot as well as an unknown schema version.

The test state of 2026-11-16 (262 tests) is in `Device-Management.md`; the numbers in this table apply to the state of 2026-11-11.

## Execution on the Pi

Cross-build with `cargo-zigbuild` 0.24.1 and Zig 0.13.0:

```sh
cargo zigbuild --workspace --tests --locked \
  --features security,zorvinkeld/ble-server \
  --target arm-unknown-linux-gnueabihf.2.28
```

The 18 generated test programs were executed under the user `pi` from a temporary directory: 90 tests passed, no failure. Real Linux file access, reopening, registration concurrency, device binding, private identity storage, Noise handshake, replay rejection and session block are also checked. Network and radio are simulated by these tests. GUI and compile-fail doctest are not part of this Pi run. The ARMv6 GATT service was also built and started.

## Limits

The described GUI and CLI simulations use simulated devices and networks. The secured status flow uses real Noise authentication over an in-memory transport. No check changes real networks or uses radio. Persistence tests use real temporary files on the local file system. In simulation mode, client and service run a single status flow. The GUI exists as a simulation. Registration and status as well as local administration are natively connected; secure network changes are missing. Windows with all features and the Linux security core are cross-type-checked. Native compiler builds on Linux/Windows and CI runs are not executed. The ARMv6 BLE cross-build uses a D-Bus sysroot of the Pi; Linux GUI and Windows runtime remain unchecked. No complete MVP system acceptance.

No evidence for power failure, competing processes or a backwards-jumping system clock. The Linux file check ran on the existing Pi file system; other file systems remain unchecked. The Unix identity store is connected with key generation, pair check and Noise handshake. Access protection uses file permissions, not disk encryption. These original workspace checks do not check operating-system key stores. Calls with client identifiers do not replace a cryptographic proof of identity. The local Linux administration socket checks the kernel user ID; the original workspace checks do not prove this runtime integration.

Build scripts: macOS ARM64, Rust/Cargo 1.99.1; client release and app package as well as ARMv6 release built with loaded Raspbian D-Bus packages. Packaging needed access to macOS system services in this environment. The new server programs were not executed on the Pi.

Server build script: ARMv6 and ARM64 built successfully on macOS ARM64 with Rust/Cargo 1.99.1, Zig 0.13.0 and cargo-zigbuild 0.24.1; D-Bus packages downloaded and checksums checked. ARM64 not executed on a target device.
