# ADR-034 – Revert and Persist

**Status:** Proposed. Supplements ADR-025 (restart options): the device now reports whether the active configuration differs from the one that applies after a restart, and a change can switch back at once.

## Context

ADR-025 lets a client choose what the device restores after a restart, but the client could not tell whether the running configuration had been persisted, and the only way back to the persisted state was a restart.

## Decision

- `RestartPolicyRead` answers with a second payload byte: `1` if the active configuration differs from the one valid after a restart, otherwise `0`. The one-byte answer of older devices reads as "same".
- The device worker keeps the comparison up to date. With a default set, the configuration differs if the active network is not the default. Without a default, it differs if the connection is one of the service's own volatile profiles.
- New restart change `Revert` (code 2) in `RestartPolicySet`: switch at once back to the persisted configuration. Volatile profiles are dropped; saved networks and the default are applied again.
- The client offers "Revert" and "Persist" exactly while the device reports a difference. "Persist" is `CurrentAsDefault` and always sets a default; "Revert" is `Revert`.

## Consequences

- A new client against an old device service shows no buttons (the byte is missing). An old client against a new service cannot read the two-byte answer.
- Persist always sets a default, so the GUI can no longer clear a default or choose "keep later changes" (`LastAccepted`); the requests still exist in the protocol.
- A revert can cut the IP connection to the device; Bluetooth stays available.
- Not proven: a real Wi-Fi change with revert on a device. The logic is covered by tests and the Linux worker tests run on a Raspberry Pi 4.
