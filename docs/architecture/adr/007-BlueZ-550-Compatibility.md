# ADR-007 – BlueZ 5.50 compatibility

**Status:** Accepted  
**Date:** 2026-11-11

## Context

On acquiring a GATT notification, BlueZ 5.50 supplies `MTU` instead of `mtu`, and it supplies no MTU for value requests. BlueR 0.17.4 requires `mtu` throughout; the public BLE test service therefore fails on the existing Pi.

## Decision

BlueR 0.17.4 is included under `vendor/bluer` with unchanged licence and a limited change to the GATT parameter evaluation. The patch accepts both spellings. If the MTU is missing in value requests, the ATT minimum of 23 applies conservatively. Acquiring a notification still requires an explicit MTU. Wrong types and values below 23 are rejected; the device assignment remains required.

The test service additionally offers `write-without-response`. The client still writes with response. This way BlueZ 5.50 acknowledges the ATT acceptance without waiting for the `WriteValue` callback path that hangs there. This does not confirm processing by the application; the complete protocol response remains authoritative.

Before each connection, the macOS client renews the native handle of the device already found. Closed old notification sessions are removed without disconnecting a connection to the same peer that has been set up in the meantime.

## Consequences

BlueZ and the operating system need not be replaced for this fix. The patch needs its own regression tests and a check on target hardware. With a suitable upstream version the local change is removed. UUIDs, frame format, session limits and authorisation rules stay unchanged. The additional write property supplements ADR-004. Radio acceptance and search remain separate check points.
