# ADR-008 – Event-based BLE search

**Status:** Accepted for P3; time limits provisional  
**Date:** 2026-11-11

## Context

The Pi announces the service with a 1.28-second advertising interval. macOS sometimes reports matching devices only after more than five seconds. A single snapshot after five seconds can therefore end the search without a hit.

## Decision

The client subscribes to search events before the scan starts and collects matching devices from this search run. Service identifiers are checked both from events and from the associated device properties. Devices are deduplicated by search handle; at most 32 entries are kept.

Without a hit, the search waits at most 20 seconds. After the first hit, one second remains for collecting further hints; later reports do not extend this period. The entire scan procedure including operating-system calls is limited to 25 seconds. The subsequent scan stop has at most five seconds and is attempted even after errors.

## Consequences

Early hits are shown quickly; delayed hints no longer lead to an empty list after only five seconds. A search handle remains a technical selection value without any trust effect. The investigation does not justify a permanent change of global Bluetooth controller values. Other radio conditions and client platforms need their own system checks.
