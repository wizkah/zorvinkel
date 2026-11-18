# ADR-024 – Session renewal, transport switching and correlation proof

**Status:** Accepted. Supplements ADR-023 (sessions, connection choice); its statements on session end and on the BLE-to-IP switch without a correlation proof are thereby replaced.

## Context

The security profile limits a session to 600 s and 1024 records per direction (`security/channel.rs`). The limit protects key material and nonce space from unlimited use and cannot be lifted. In GUI operation (status query every 5 s) the connection therefore ended after ten minutes without notice. In addition, an IP session ended on a Wi-Fi switch (new address) without a fallback, the BLE-to-IP switch did not use the one-time proof required in Architecture §5, and the GUI did not show the connection path.

## Decision

- **Session renewal.** The client reports the remaining session time (too few remaining records count as expired). The GUI renews the session by itself and unobtrusively 45 s before the end (new connection set-up to the same device, new keys). The user can switch this off per device in the status area ("Renew session automatically", default: on, not saved permanently). If it is off, the GUI shows a hint with "Renew now" from 120 s of remaining time; an expired session is explained as its own error (`SessionExpired`).
- **Fallback to Bluetooth.** If an IP session breaks off (e.g. Wi-Fi switch), the client reconnects to the same device over Bluetooth, provided it was found there. This does not apply to identity and permission errors and not to expired sessions (these are renewed). The running request counts as failed, except for the background refresh and jobs that change the device's network (connect Wi-Fi, switch device Wi-Fi on or off): for these the end of the IP session is expected, the operation is completed after the reconnection over Bluetooth using its operation identifier and only then reported as a success. Afterwards the client may switch back to IP as soon as reachability is confirmed.
- **Correlation proof.** New operations `CorrelationIssue` (17) and `CorrelationRedeem` (18), capability bit 1024 (only with an IP endpoint). In an authenticated session the device issues 256 random bits, binds them to the client key and holds at most 8 open proofs. The client redeems them within 30 s in the new IP session; the device accepts only the same client key, only over the IP endpoint and only once (comparison in constant time). Without confirmed redemption or without the capability bit the client stays on Bluetooth.
- **Display.** The connection path (Bluetooth, network (IP), simulation) appears in the tab header, in the tile and in the status area. A switch from IP to Bluetooth is reported.

## Consequences

- Each renewal performs a complete connection set-up (handshake, capabilities, reading the state); over BLE on the Pi Zero W this takes noticeable time. Renewal runs in the background; until then the old session remains usable.
- With renewal switched off, the connection ends after ten minutes; this is intended.
- After a fallback, the state of the interrupted request is reliably known only by reading again.
- Devices without bit 1024 are no longer switched from BLE to IP until the device service is updated.
