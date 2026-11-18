# ADR-025 – Restart options and rollback of the volatile device Wi-Fi

**Status:** Accepted. Replaces in ADR-017 the statements on the restart choice of the device Wi-Fi and on the exempted 60-second rollback.

## Context

The requirements call for the restart options `LastAccepted` and `DefaultProfile` for Wi-Fi connection and device Wi-Fi. So far only the simulation knew both; on the device only the "keep" choice (`WifiPersistence`) applied. A volatile device Wi-Fi was not rolled back, although a volatile Wi-Fi change is rolled back after 60 s without a confirming client (Architecture §6).

## Decision

- Two new authenticated operations: `RestartPolicyRead` (19, response `RestartPolicy`) and `RestartPolicySet` (20, with operation ID and change `LastAccepted` or `CurrentAsDefault`, progress via `WifiPoll`). Capability bit 2048. Both network stacks report the capability.
- The option lives in the persistent device state (`settings.restart_policy`). A default holds the type (Wi-Fi or device Wi-Fi) and, for Wi-Fi, the SSID; credentials stay in the network stack or in the protected storage of the device Wi-Fi (ADR-026).
- `LastAccepted`: after the restart the device restores what a client had it keep ("keep" chosen: saved Wi-Fi or switching the device Wi-Fi on again through the marker `access-point-restart`). Without "keep", the original configuration of the device applies.
- `DefaultProfile` through "current setting as default": if a Wi-Fi is active, it stays saved (a profile created by the service is secured by `SAVE_CONFIG` or NetworkManager, an existing one is left unchanged). If the device's own Wi-Fi is active, it is recorded as the default; a foreign access point cannot be a default. Without an active connection it is rejected (`NoActiveNetwork`).
- The default takes precedence: as long as it is set, new Wi-Fi connections and the device Wi-Fi are not kept (volatile); the default restores itself after every service or device restart and after every rollback of an unconfirmed change. To do so, the Wi-Fi is searched by SSID among the saved profiles (profile numbers are not stable), and the device Wi-Fi is started from the saved credentials. This restoration is not journalled, because it only repeats a configuration that has already been decided.
- Volatile device Wi-Fi: it follows the rollback of the Wi-Fi change. If no authenticated client contacts the device again within 60 s, or it is lost three times in a row, it is rolled back (saved configuration, or the default if one is set). The state survives a service restart. If the device Wi-Fi is the default or "keep" is chosen, it is not rolled back.

## Consequences

- Other devices that use the volatile device Wi-Fi without an open client lose it after 60 s if no client confirms the state. Whoever wants to offer it permanently chooses "keep" or sets it as the default.
- "Keep" and default are separate switches; the GUI explains the precedence. A default from an existing (not service-created) profile requires that this profile is saved; the service does not check that but only finds it by SSID.
- Proof on a real wpa_supplicant, NetworkManager and restart is missing; the logic has been checked with simulated network stacks.
