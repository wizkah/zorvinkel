# ADR-020 – Authenticated IP reachability test

**Status:** Accepted; the restriction to `Authenticate` is superseded by [ADR-023](023-IP-Channel-And-Network-Discovery.md)

## Context

A Wi-Fi login of the device does not prove that the computer reaches it over IP (client isolation, separate networks, firewall). The interface should distinguish the two and make sure that the stored device really answers at the address. BLE setup must not depend on this.

## Decision

- The device service listens on TCP port 47470 (dual stack, otherwise IPv4). Capability bit 128 reports the endpoint only if it could be bound; without an endpoint the service continues unchanged.
- The endpoint runs the same Noise XX handshake and the same registration check as BLE, with its own context (`…/ip`) in the prologue. Records are framed with a 4-byte length; the length is limited before allocation.
- IP sessions answer only `Authenticate`. Registration, setup and all other operations stay restricted to BLE; other operations return `Unsupported`.
- Limits: at most four simultaneous connections, at most two IP sessions out of four session slots (BLE stays reachable), 15 s total duration per connection.
- The client evaluates the global addresses from `SystemInfo` and checks them in parallel (3 s each). An address counts as successful only if the handshake proves the stored device key and `Authenticate` is confirmed. Result `Authenticated` as soon as one address passes; `Unreachable` if all fail; `Unverified` without the capability, system information or an address.
- The client checks on connecting and afterwards at most every 30 s or when addresses change; the result appears as "Reachable from this computer" in the device tab.
- The systemd unit allows `AF_INET` and `AF_INET6` for this.

## Consequences

A failure blocks no setup. The endpoint is reachable on all interfaces, including the device Wi-Fi, and reveals only the handshake to unauthorised peers; a firewall or interface binding is not planned. The check uses only addresses reported by the device; name resolution (mDNS) and reachability from other networks are missing. The client does not distinguish a rejected authentication from a network error; both appear as "Not reachable".
