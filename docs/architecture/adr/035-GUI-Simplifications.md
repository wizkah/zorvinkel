# ADR-035 – GUI simplifications: renewal, saved passwords, connection lamps

**Status:** Accepted. Replaces in ADR-024 the per-device switch for session renewal and the "Renew now" hint. The restart choice is ADR-034, the terminal login ADR-033.

## Context

Three details of the GUI asked the user to decide things the client can decide itself, or showed the way to a device only indirectly.

- ADR-024 let the user switch automatic session renewal off per device. With it off the connection simply ended after ten minutes, which nobody wants; the switch and its expiry hint only added surface.
- Saving a Wi-Fi password had two buttons: the one for the system's credential store and "Use cached password" for the password Zorvinkel remembered itself. The user cannot tell which of the two holds the password.
- The device tile showed the connection path only as text, and not how the latest search had found the device.

## Decision

- **Renewal is always on.** The GUI renews the session 45 s before its end as in ADR-024. The switch, the expiry warning and "Renew now" are gone. An expired session (for example after the device was away) is still reported as `SessionExpired` and reconnected.
- **One saved-password button.** It reads the system's credential store first and, if nothing is found there, the password Zorvinkel remembered. The request `ReadCachedCredentials` is removed; the stores themselves are unchanged.
- **Two lamps on the device tile,** "BLE" and "IP". A lamp is on when the latest search found the device that way (`DiscoveredDevice::via_ble`, `via_ip`). They show how the device was found, not the path of the open session; that stays in the tab header (ADR-024).

## Consequences

- The user can no longer keep a session from being renewed. A device that is busy with a long set-up gets a renewal in the background like any other.
- Which store supplied a saved password is no longer visible.
- Both lamps off means the latest search did not find the device; the tile can still belong to an open session.
- Checked by build, clippy and tests only; not looked at on screen and not tried against a device.
