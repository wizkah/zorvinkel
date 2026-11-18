# ADR-006 – Security profile for the status flow

**Status:** Accepted for implementation and simulation; no product release  
**Date:** 2026-11-11

## Context

Registration and status need a binding message contract and a cryptographic proof of possession of both identities. TOFU and keys deposited during setup must use the same secured connection.

## Decision

### Cryptography and identity

`snow` 0.10.0 with explicitly selected RustCrypto features and the default resolver implements `Noise_XX_25519_ChaChaPoly_SHA256`. Static public and private keys are 32 bytes each. Keys are generated through the library generator with operating-system randomness; loaded pairs are checked by deriving the public key in the library. Private keys stay behind opaque handles and outside the general device state.

The Noise prologue contains `zorvinkel/status/1.0/` and the channel name `simulation`, `ble` or `ip`. Different protocols and channels must not complete a shared handshake. Handshake payloads are empty. The three XX messages are 32, 96 and 64 bytes. The whole handshake has ten seconds; an error or a clock reversal closes it.

The client checks the static device key after message 2, before it sends message 3 with its identity. A new TOFU binding is stored persistently only after the handshake has completed locally. An existing binding is never replaced automatically. On the device the handshake alone grants no administrative rights.

### Records and messages

Each handshake or transport message gets a four-byte big-endian length. A complete record including the length is at most 4096 bytes. Empty, incomplete, oversized or concatenated records are rejected. Handshakes and encrypted records remain separate phases.

Encrypted application messages have the following header; multi-byte numbers in the header are little endian:

| Offset | Length | Content |
|---|---|---|
| 0 | 2 | Magic `ZV` |
| 2 | 1 | Major version `1` |
| 3 | 1 | Minor version `0` |
| 4 | 8 | Request ID |
| 12 | 2 | Operation |
| 14 | 2 | Payload length |
| 16 | variable | Typed payload |

Only version 1.0 is supported. Request IDs start at 1 per session and increase without gaps; only one request per client is open at a time. Responses must match the identifier and operation of the open request. Unknown operations are rejected explicitly. Invalid messages close the session before execution.

| Operation | Code | Request | Success response |
|---|---|---|---|
| Register | 1 | Empty | Empty, response code `0x8001` |
| Authenticate | 2 | Empty | Empty, response code `0x8002` |
| Network status | 3 | Empty | Response code `0x8003`: mode byte, SSID length as a byte, UTF-8 SSID |
| Capabilities | 4 | Empty | Response code `0x8004`: four-byte bit mask, little endian |
| Error | `0xffff` | None | One byte: 1 unauthorised, 2 registration closed, 3 unsupported, 4 service error |

Status mode: 0 disconnected with empty SSID, 1 Wi-Fi, 2 device Wi-Fi. SSIDs are at most 32 bytes and contain no control characters. Bits 0 and 1 of the capabilities denote status retrieval and registration/authentication. Network changes, key export and transport handover are not offered. A Wi-Fi login does not confirm IP reachability of the client.

### Authorisation, freshness and lifetime

Registration uses only the client key proven in the handshake. The registration decision and commit happen before the response. Known clients can authenticate again after a lost response; a closed registration stays closed. Before status and capabilities, a successful registration or authentication in the same session is required. Before every further action the current registry is checked. A revocation additionally ends all associated active sessions through the shared session management; native revocation entry points must use this combined flow.

Noise manages nonces and ordering. Tampering, replay or wrong ordering close the session; a new connection needs a fresh handshake. Transport states are neither exported nor stored. At most 1024 records per direction and ten minutes of session lifetime are allowed; after that a new handshake is required. The shared session management allows at most four active sessions. A session that is not yet authorised expires after ten seconds; the native runtime must check waiting sessions for expiry regularly. Application buffers are limited to one bounded message per direction. Time is supplied monotonically by the runtime.

The simulated transport needs no sockets. The later BLE integration must deliver complete bounded records and close the same session on fragmentation errors. The public P3 test service stays separate. A running BLE-to-IP handover is not available; it needs a separately defined one-time correlation inside both secured sessions. Standalone IP sessions must not take over running BLE operations.

## Consequences

The security adapter uses existing cryptography instead of custom primitives. Private keys and temporary plaintext buffers are erased on release, as far as the own code controls them. Security-critical errors are reported without key or message contents.

Limits are cautious implementation bounds, not measured resource budgets. Native connection management, load testing, operating-system key stores and the local administrator permission check remain prerequisites of system acceptance. The package review covers the published API, the selected resolvers and the known nonce advisory [RUSTSEC-2024-0011](https://rustsec.org/advisories/RUSTSEC-2024-0011.html), whose fixed version range includes 0.10.0. This is not a full review of all dependencies. Snow and the integration are not considered audited without an independent review. TOFU keeps the accepted first-contact limit.

References: [Noise specification](https://noiseprotocol.org/noise.html), [Snow 0.10.0](https://docs.rs/snow/0.10.0/snow/), [Snow project](https://github.com/mcginty/snow).
