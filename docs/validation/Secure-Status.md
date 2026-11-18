# Secured status flow

**Date:** 2026-11-11

## Environment and scope

macOS ARM64, Rust/Cargo 1.99.1; additionally precompiled security and persistence tests on a Raspberry Pi Zero W with Raspbian 10. Real Noise XX key generation and encryption over a simulated in-memory transport. No BLE or IP management service and no change to real networks.

```sh
cargo run --locked -p zorvinkel-client --features security -- --secure-simulate
```

The flow registers a client, closes registration and, after a fresh handshake and renewed authentication, reads status and capabilities. The demo keeps data in memory only.

## Results

| Check | Result |
|---|---|
| Security integration tests | 19 passed |
| Generated identities and registry after reopening | One persistence test passed |
| Response correlation and tampered response | Two unit tests passed |
| Versioned message contract | Four codec tests passed |
| Entire workspace, all features | 93 tests passed |
| Build and Clippy, all features, macOS | Passed |
| CLI demo | Registration, renewed authentication, status and capabilities confirmed |

Checked are concurrent first registrations, closed registration, key authorisation during setup, wrong device keys, missing authorisation, commit failure, lost response, restart and client loss. Private keys come from the library generator; the file storage is reopened with these keys and used for a further handshake.

Tampered requests and responses, replay within and outside a session, mismatching request IDs, unknown operations, incompatible versions, wrong channel context, damaged records and invalid key pairs are rejected. Deadlines, clock reversal, record and session limits and the immediate session block after revocation are checked. Status providers are not called before authorisation. The status contract confirms no IP reachability of the client.

## Library review

Snow 0.10.0 uses only the explicitly selected RustCrypto features for Curve25519, ChaChaPoly, SHA-2 and operating-system randomness. The known advisory [RUSTSEC-2024-0011](https://rustsec.org/advisories/RUSTSEC-2024-0011.html) concerns an unchecked increase of the receive nonce; the fixed version range includes 0.10.0. In addition, the adapter closes every session after a cryptographic error.

The API, key derivation, selected resolvers and error paths were checked against the loaded package source and the tests. The [Snow note on the missing formal audit](https://github.com/mcginty/snow) remains. There was no complete automated vulnerability check of all transitive dependencies and no independent review of the integration.

## Platform checks

On the macOS host the Rust target libraries for Windows GNU x86-64 and Linux GNU ARM64 are installed.

```sh
cargo check --workspace --target x86_64-pc-windows-gnu --all-features --all-targets --locked
cargo check --workspace --target aarch64-unknown-linux-gnu --features zorvinkel-adapters/security --all-targets --locked
```

Windows with all features and Linux without native BLE/GUI dependencies pass the cross type check. No target programs are linked or executed in the process. The full Linux check with all features was attempted and fails at `libdbus-sys`, because pkg-config is not set up with a Linux D-Bus sysroot. The Linux GATT server is therefore still not type-checked. Native CI runs were not executed without a configured Git remote.

## Limits

This experiment checks the in-memory transport; it proves no radio, GUI or operating-system integration. The simulated deadlines are no measurement of the target hardware. Independent security review, Linux/Windows key stores, BLE-to-IP handover and full system acceptance remain open.
