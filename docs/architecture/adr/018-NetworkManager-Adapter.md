# ADR-018 – NetworkManager adapter

**Status:** Accepted

## Context

ADR-015 prefers NetworkManager, but the adapter was missing so far. The device service runs without rights (user `zorvinkel`, group `netdev`). The adapter must support the same network jobs, restart options and device Wi-Fi (ADR-017) as the wpa_supplicant fallback.

## Decision

- The adapter talks to NetworkManager over the system D-Bus interface (crate `dbus`, feature `network-manager`; without the feature the service reports the limitation instead of starting). The rules sit behind a small interface (`NetworkManagerApi`) and can be tested without NetworkManager. The common interface `StackControl` lets the setup flow serve both stacks the same way.
- Credentials reach NetworkManager only over D-Bus, never through process arguments or files of the service.
- Own profiles are named `zorvinkel-<identifier>` and are created in memory only (`AddConnection2`, without autoconnect start). Volatile is the default; the "keep" choice saves only the active own profile (`Update2` with "to disk") and then releases it, so later clean-up does not delete it. Older saved profiles of the service for the same network are replaced. Foreign profiles are never changed or deleted.
- Rolling back volatile changes deletes all unsaved `zorvinkel-` profiles and releases the device's autoconnect; NetworkManager then chooses a saved network. Orphaned profiles of an earlier service run are removed before new ones are created.
- Rollback activates the previous profile or disconnects the device. Switching off the device Wi-Fi first disconnects (which blocks autoconnect), deletes the own profile and leaves reconnecting to the caller.
- Device Wi-Fi: WPA2/CCMP, channel 6, `ipv4.method=shared` with `192.168.4.1/24`. NetworkManager handles address and DHCP itself; the root helper from ADR-017 is omitted. The function is offered only if the device reports AP mode.
- The service has no local session, so without its own rule NetworkManager refuses it `AddConnection2` ("Insufficient privileges"). The installer creates `/etc/polkit-1/rules.d/50-zorvinkel.rules`: the user `zorvinkel` may `network-control`, `settings.modify.system`, `wifi.scan` and `wifi.share.protected`, nothing beyond that. The group `netdev` alone is not enough.
- Under NetworkManager the status for the client comes from the same interface instead of from `wpa_cli`.
- NetworkManager checkpoints are not used; the service's journal and rollback apply as with the fallback.

## Consequences

The adapter has been tested only against a simulated interface. Open are operation on a real NetworkManager (Raspberry Pi OS), the effect of the polkit rule (the right was missing on the Pi 4, and the rule is not installed there yet), the need for `dnsmasq-base` for `shared` and the behaviour of autoconnect after disconnecting. The credentials of the device Wi-Fi are still in plain text in the service storage (ADR-017); NetworkManager manages saved station profiles itself.
