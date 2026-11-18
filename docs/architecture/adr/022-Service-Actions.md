# ADR-022 – Starting, stopping and restarting services

**Status:** Accepted

## Context

The services configured in ADR-021 should be startable, stoppable and restartable from the client. The device service runs unprivileged with `NoNewPrivileges`; it must receive neither general root rights nor arbitrary commands, and a client must not be able to lock itself out.

## Decision

- The operator allows actions per service in `services.d` (`actions = ["start", "stop", "restart"]`). Without the key the service stays display-only. The permitted actions belong to the service list (`Services`).
- New authenticated BLE operation `ServiceAction` (code 16, capability bit 512) with action and service name; response `ServiceActionAccepted` (0x800f) only confirms acceptance. The client reads the service list again shortly afterwards; the state appears as it is. If the request is unknown or not permitted, the service answers with `Conflict`, with too many open requests with `Busy`, and without a helper with `Unsupported`.
- As with ADR-016, a root-owned helper does the execution: the device service files a request file (`<action> <unit>`) under `/run/zorvinkel/service-requests/`, a systemd path unit (`zorvinkel-service.path`) starts `zorvinkel-service.service`, and this calls `zorvinkeld --run-service-requests`. The helper treats the files as untrusted (regular files up to 256 bytes only, no symlinks), reloads the root-owned configuration, executes only the action and unit allowed there, and deletes every file before execution. It calls `systemctl --no-block <action> -- <unit>`. At most eight open requests.
- Without the installed path unit (`install.sh` enables it) the service reports neither capability bit 512 nor permitted actions; the list stays display-only.
- Protected units must never receive actions, so that access is preserved: `bluetooth`, `dbus`, `ssh`/`sshd`, `networking`, `NetworkManager`, `wpa_supplicant`, `dhcpcd`, `rescue` and all units with the prefix `zorvinkel`, `systemd-`, `getty@`, `serial-getty@`. The configuration discards such files with a note in the journal. Service names must be unique; a duplicate name is skipped.
- The interface shows the permitted actions per service. Stopping and restarting require a confirmation, starting does not.

## Consequences

Every registered client may trigger the permitted actions of all configured services (equal rights in the MVP, see Architecture §4). The device service receives no new right; the right lies in the helper, which executes only defined combinations. Whoever may write `services.d` thereby decides which services clients can control. A successful response only means "accepted": whether `systemctl` executes the action or the service starts is shown only by the following state display; errors of the helper are in the journal (`zorvinkel-service.service`). The order of several requests follows the file names.
