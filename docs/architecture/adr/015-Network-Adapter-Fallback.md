# ADR-015 – Network adapter and fallback

**Status:** Accepted

## Context

Devices use different network stacks. The existing wpa_supplicant/dhcpcd adapter so far supports only volatile Wi-Fi connections.

## Decision

NetworkManager is preferred. If it is missing, the device service checks wpa_supplicant/dhcpcd as a fallback. The service checks responsibility and capabilities and reports them to the authenticated client. The client offers only available actions. Both adapters are to support the persistent MVP network functions including restart options.

## Consequences

The NetworkManager adapter is described in ADR-018. The fallback still needs persistence, secret protection, AP and DHCP services and recovery. Selection and capabilities must be checked for both stacks. ADR-011 and ADR-012 describe the previous volatile state; their limitations remain valid until implementation.
