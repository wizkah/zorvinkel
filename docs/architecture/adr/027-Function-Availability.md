# ADR-027 – Function overview per connection

**Status:** Accepted

## Context

The requirements call for showing which functions are available over the current connection (Architecture §6 and §7: check capabilities before every action, show unavailable functions in an understandable way). So far the GUI hid missing pages without a hint.

## Decision

- The application layer derives from the authenticated reported device state (`SetupSnapshot::functions`), for eight functions, whether the device offers them: set up Wi-Fi, device Wi-Fi, restart option, system information, show services, control services, shut down or restart the device, open a terminal. The terminal is available once the reported system information contains a routable address for `sshd` (ADR-030); it depends on that address, not on a device capability, and the hint says that no address was reported yet.
- The GUI shows the list in the status area "Functions over this connection" with the offer or the hint to update the device service. Pages appear only for offered functions. Wi-Fi connection, device Wi-Fi and restart choice form one "Wi-Fi" page with a switch; the restart choice (original or keep, the latter with the check box "Also adopt later changes": on = `LastAccepted`, off = fixed default) appears there once.
- BLE and IP carry all operations (ADR-023); the connection path therefore does not change the list and is shown next to it in the status area. A device that reports the capability can still reject the action at execution time; the service checks again (Architecture §6).

## Consequences

- The overview describes capabilities, not the momentary availability of individual jobs (e.g. "busy").
- A function that would be possible over one transport only needs a further state in `Availability`.
