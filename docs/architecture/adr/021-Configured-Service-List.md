# ADR-021 – Configured services in the device tab

**Status:** Accepted

## Context

Other services run on the device (e.g. Raspotify) whose version and state the user wants to see in the client, some with their own web interface. The selection should be made by configuration on the device, without the client sending commands or any code being loaded dynamically.

## Decision

- The operator creates one TOML file per service in `/etc/zorvinkel/services.d/` (format and limits: `packaging/Services.md`). The service reads the directory anew on every request, validates strictly (unknown keys, lengths, characters, service name, port, path) and skips invalid files with a journal entry. At most 16 services.
- New authenticated, purely read-only BLE operation `Services` (code 15); response `Services` (0x800e) with a fixed, bounded binary structure per service: group, name, optional version, state (`Unknown`, `Active`, `Activating`, `Inactive`, `Failed`) and optional web link (scheme, port, path, optional label). Capability bit 256 reports the function as soon as the configuration directory exists. As with ADR-019 there is no version field, because client and service are delivered together.
- The state comes from a single `systemctl show` call for all services. The version is supplied by a configured command without a shell (time and output limit, result remembered for ten minutes); `version_word` selects one word of the first output line.
- The web link contains no host. The client assembles the URL from a reported device address (IPv4 preferred, IPv6 in brackets) and shows a button (system browser) and a QR code. If IP reachability is not confirmed (ADR-020), a hint appears; the link is offered anyway, because the QR code is meant for another device.
- Each group becomes its own tab in the device tab (alphabetical); services of a group are listed one below the other. The group is purely presentation.
- Starting, stopping and restarting the services is not part of this decision; it follows in ADR-022.

## Consequences

Every registered client receives names, groups, versions, states and web addresses (without host) of the configured services, but no commands. Whoever may write `services.d` can have programs executed as the service user through `version_command`; the files belong to `root`. The service starts one `systemctl` process per refresh; version commands run synchronously in the service (at most 2 s per command and ten-minute window). The web interface is neither protected nor checked by Zorvinkel. The device service depends on the crates `toml` and `serde` (feature `ble-server`).
