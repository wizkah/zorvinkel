# ADR-016 – Shutting down and restarting the device

**Status:** Accepted

## Context

A connected client should be able to shut down or restart the device. The device service runs unprivileged with an empty capability set and `NoNewPrivileges`; restarting the operating system requires root rights.

## Decision

- New authenticated BLE operation `Power` (code 9) with the commands `Shutdown` and `Restart`; response `PowerAccepted`. Capability bit 16 reports the function.
- The response only confirms acceptance. The service files the request after a short delay so that the encrypted response reaches the client. A second request is rejected with `Busy` until then.
- To do so, the service only creates an empty file `poweroff` or `reboot` in its runtime directory. Root-owned systemd path units (`zorvinkel-power@.path`/`.service`) then run `systemctl --no-block <action>`. The service gets no additional rights and needs neither polkit nor sudo.
- The service reports the capability only if the installer has enabled both path units.
- The client asks explicitly before execution and ends the connection afterwards.

## Consequences

Every registered client may switch the device off (equal rights in the MVP). After shutdown the device can only be switched on again by disconnecting it from power or at the device; the interface points this out. The function has not yet been checked on a Pi.
