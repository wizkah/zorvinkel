# ADR-019 – Device system information

**Status:** Accepted

## Context

The client should show the identity and state of the device (device model, operating system, hostname, IP addresses, load, uptime, memory, disk space, version of the device service) without needing a terminal.

## Decision

- New authenticated, purely read-only BLE operation `SystemInfo` (code 14); response `SystemInfo` (0x800d) with a fixed, bounded binary structure (version of at most 32 visible ASCII characters, uptime, load ×1000, memory and disk space in bytes; optionally SoC temperature, current and maximum CPU clock, firmware flags for under-voltage/throttling, device model, operating system, kernel, hostname and at most eight global IP addresses with interface; texts of at most 64 characters without control characters). The structure is hard-coded; extensions change the payload without a version field, because client and service have so far been delivered together. Capability bit 64 reports the function; the protocol discards implausible values (more free than total).
- The Linux adapter reads `/proc/uptime`, `/proc/loadavg`, `/proc/meminfo`, `/sys/class/thermal/thermal_zone0/temp`, `/sys/devices/system/cpu/cpu0/cpufreq`, `/etc/os-release`, `/sys/firmware/devicetree/base/model` and calls `df -Pk /`, `ip -o addr show scope global` and `vcgencmd get_throttled`. The version is the package version of the device service. Missing optional sources do not prevent the other values. The service user may not read `/dev/vcio` (group `video`); without an additional group the throttling flags are missing.
- The client reads the values on connecting and on every status update and shows them in the device tab on the "System" page.

## Consequences

Every registered client may read the values. They contain no secrets but reveal version, load, hostname and IP addresses. The serial number is deliberately not transmitted. Disk space refers to the root file system only. `df` is called synchronously in the service.
