# ADR-014 – Device names when names collide

**Status:** Accepted  
**Date:** 2026-11-12

## Context

The hostname should appear without a permanent identifier suffix. Two registered devices with the same name must remain distinguishable even when one of them is unreachable.

## Decision

The native client removes the previous four-character BLE name suffix from the display. After successful registration and authentication it stores hostname, search handle and public device key as separate protected display metadata.

An identifier appears only when different registered device keys carry the same hostname. Several handles of the same identity do not count as different devices. The identifier initially uses eight hexadecimal characters of the public key and is extended on a collision.

The metadata influence neither device selection nor the trust check. Missing or damaged display metadata do not replace a trust binding; the display falls back to the hostname.

## Consequences

Devices registered but offline are taken into account for name conflicts. Existing registrations receive their name metadata at the next successful connection. The device name remains an unsecured display and is no proof of identity. The display decision changes no radio address, device key or registration.
