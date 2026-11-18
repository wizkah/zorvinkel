# ADR-009 – Native secured BLE status service

**Status:** Accepted; macOS identity storage superseded by [ADR-010](010-macOS-Client-Keychain.md)  
**Date:** 2026-11-11

## Context

The encrypted status contract needs a native transport, persistent device binding and a separate local administration entry point. The existing Pi uses wpa_supplicant without NetworkManager.

## Decision

The secured service uses its own GATT UUIDs:

| Element | UUID |
|---|---|
| Service | `719e40a0-4a4b-4d75-9c08-a9cbb88f4200` |
| Write with response | `719e40a0-4a4b-4d75-9c08-a9cbb88f4201` |
| Notifications | `719e40a0-4a4b-4d75-9c08-a9cbb88f4202` |

Advertising sets an unlimited visibility duration, independent of the adapter default.

The BLE frame format gets message type `1` for complete security records. Type `0` stays reserved for the public test service. Both endpoints discard the other type. Fragment size and size limits remain. Native time sources are monotonic; waiting handshakes and sessions are checked regularly. Closed security sessions lose their authorisation immediately; one second is left for the rejection response before the radio link is actively torn down. When the service ends, it waits for the deregistration from GATT and advertising.

Device and Unix client open existing identities read-only. Only an explicit device initialisation creates the first identity and registry. A new client store is created on first start. A persistent device directory remembers search handles already selected; a deleted binding record is not replaced automatically. Deposited device keys are checked before the client identity is disclosed. Display name and search handle remain unauthenticated selection aids.

The Linux service runs as `zorvinkel`. A Unix socket accepts a targeted grant for five minutes, key authorisation, revocation and output of the public device key. Every access needs a UID 0 confirmed by the kernel. Revocation first stores the block and then ends the affected sessions. The socket is not a public BLE command.

On the existing Pi, network status is read only through `wpa_cli status`. The query has time and output limits; its local socket is in the service directory. A missing or unintelligible status is treated as unavailable. This observation performs no network changes and does not replace a NetworkManager adapter.

## Consequences

The native client can find devices, register, authenticate again and read their network status. The interface blocks network actions that are not yet connected. A successful Wi-Fi status does not confirm IP reachability.

The first Unix client uses protected files; operating-system key stores remain open. Bindings follow the full search handle. A changed handle cannot be assigned automatically to a stored device; a previously deposited key remains the secure setup path. Unsupported SSID encodings provide no status for the time being. NetworkManager integration, Windows persistence, independent security review and load acceptance remain outstanding.
