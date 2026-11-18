# ADR-023 – IP channel with all operations and device search via mDNS

**Status:** Accepted; session end, fallback and correlation proof are supplemented or replaced by ADR-024. Replaces the restriction to `Authenticate` from ADR-020; its other decisions (port, handshake, connection count limits) continue to apply.

## Context

ADR-020 made the IP endpoint a pure reachability test. If a network connection exists, however, the client should handle all information and actions over it without Bluetooth having to be in range. It should also find devices on the local network in a manner similar to Bonjour, even without a Bluetooth adapter.

## Decision

- **One channel, same operations.** The IP endpoint (TCP port 47470, Noise XX with context `…/ip`) serves the same operations as BLE, including `Register`, setup, device Wi-Fi, system information, services and power. The same registration states and permission checks apply; only the transport differs.
- **First contact and registration over IP are allowed.** The TOFU risk from ADR-002 remains accepted; the attacker's reach grows from "within radio range" to "on the same network". Whoever does not want that closes registration (`Closed`) or deposits keys in advance (Architecture §4).
- **Sessions.** At most two IP sessions out of four session slots and four simultaneous connections. The handshake must be completed within 15 s; an existing session is disconnected after 60 s without a request and ends at the latest with the session duration of the security profile (600 s, 1024 records). The client renews the session (ADR-024).
- **Announcement.** The device announces the endpoint via mDNS/DNS-SD as `_zorvinkel._tcp`. The instance name is the Bluetooth name (`<hostname>-<4 hex>`), the host name `zorvinkel-<instance>.local`, so it does not touch the host name of Avahi. The TXT record contains `v=1` and `fp`, the first 8 bytes of the SHA-256 of the device key. The announcement is optional; if it fails, the service stays unchanged.
- **Port.** mDNS uses port 5353 together with other participants (SO_REUSEADDR/-PORT, multicast), also with a running Avahi. The own port 47470 stays unchanged.
- **Search.** The client searches Bluetooth and mDNS at the same time; either source may be missing. `fp` is only a hint: if it matches exactly one registered device identity, that entry (and thus the same trust binding) is used, otherwise a new contact `ip:<instance>` is created. If Bluetooth already lists a device with the same name, this unregistered network entry is not shown in addition: the first registration runs over Bluetooth (radio range), and later searches merge both paths through the fingerprint. The name only hides a duplicate entry and establishes no trust. Name and fingerprint prove nothing; only the handshake proves the identity. Link-local IPv6 addresses are not used, since the announcement carries no interface.
- **Connection choice.** A connection found on the network is preferred; if it fails, the client falls back to Bluetooth. An identity mismatch is never retried over the other path. If a BLE session exists and the reachability test confirms the device, the client switches to IP (new session, same bound key, `Authenticate`, correlation proof per ADR-024); if the switch fails, BLE stays. Over IP, reachability counts as confirmed.
- The unit `zorvinkeld.service` needs no change (`AF_INET`/`AF_INET6` suffice for mDNS).

## Consequences

- A network attacker can occupy connection slots (limited to two IP sessions) and register before a user at first contact, as long as the device is `Unclaimed`.
- mDNS finds only devices in the same network segment; client isolation, missing multicast or a firewall prevent that. This is up to the user; Bluetooth remains the path without a network.
- A Wi-Fi switch over IP ends the running session because the address changes; the client falls back to Bluetooth (ADR-024).
- New dependency: `mdns-sd` (without an async runtime).
