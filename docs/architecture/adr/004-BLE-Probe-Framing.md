# ADR-004 – BLE test service and fragmentation

**Status:** Accepted for P3; limits provisional  
**Date:** 2026-11-11

## Context

BLE must be testable without a network and initially without administrative rights. Radio hardware is missing; encoding and error handling must remain testable independently of it.

## Decision

A separate GATT test service transmits only fixed public test patterns. It is active only when started explicitly with `--ble-probe`. The default build and `--simulate` start no radio access. The service connects no registration, storage or network management functions.

### GATT contract

| Element | UUID / property |
|---|---|
| Primary test service | `719e40a0-4a4b-4d75-9c08-a9cbb88f4100` |
| Client → device | `719e40a0-4a4b-4d75-9c08-a9cbb88f4101`, write with response |
| Device → client | `719e40a0-4a4b-4d75-9c08-a9cbb88f4102`, notify |

Advertising contains the service and the name `ZV-P3-<address suffix>`. Name and radio address are not proof of identity. The client searches on the first available adapter, lists at most 32 matching devices and selects through the current search handle. Higher layers must not derive trust from this.

### Frame format

A characteristic value contains exactly one fragment. Multi-byte numbers are little endian.

| Byte offset | Length | Content |
|---|---|---|
| 0 | 2 | Magic `ZV` |
| 2 | 1 | Major version `1` |
| 3 | 1 | Message type `0`: public test pattern |
| 4 | 4 | Message identifier |
| 8 | 2 | Total length of the message |
| 10 | 2 | Fragment offset |
| 12 | variable | Non-empty payload section |

Per connection and direction, identifiers start at 1 and increase without overflow. Fragments follow without gaps and in order; length and identifier stay the same. Receive buffers hold at most one message per direction. Errors close the session and discard partial messages; a new connection gets new states.

The native adapters use fixed 20-byte values with eight payload bytes. The codec supports 20 to 512 bytes per value and at most 4096 bytes per message. A started message has 30 seconds in total; further fragments do not extend the deadline.

The request pattern is 256 bytes: byte 0 is `1`, all further bytes equal their index. The response has the same structure but byte 0 equal to `2`. Other complete messages are rejected, not echoed. The client checks the response and repeats the attempt with a newly built connection.

### Adapters and limits

The client optionally uses `btleplug`, the Linux test service uses `bluer` with BlueZ and D-Bus. Runtime and libraries stay in the adapter or the program entry point. Own queue: at most 32 write events. The test service holds at most four notification sessions, ends them after 60 seconds at most and limits sending a response to five seconds.

## Consequences

Codec and probe sessions can be tested without hardware. The code offers neither encryption nor cryptographic replay protection; identifiers only detect repetitions within the same connection. BLE and operating-system buffers and the native notification lifetime need measurements.

The full security profile, administrative messages, version negotiation and MTU optimisation are thus not defined. The test service is not an unsecured management access and no substitute for hardware acceptance.

API references: [btleplug](https://docs.rs/btleplug/0.13.5/btleplug/), [BlueR GATT](https://docs.rs/bluer/0.17.4/bluer/gatt/local/).
