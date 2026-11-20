# ADR-031 – Terminal tunnel channel over Bluetooth

**Status:** Proposed (not yet implemented). Supplements ADR-030 (transport and open details).

## Context

ADR-030 forwards a byte stream of the encrypted channel to the local `sshd` when the client reaches the device over Bluetooth. The existing secure channel is unsuited as a carrier:

- A session ends after 600 s or 1024 records per direction (`security/channel.rs`, ADR-024). SSH sends small packets at once, so a keystroke and its echo each cost a record; the record limit would end a terminal session after roughly a thousand exchanges, often long before 600 s.
- A record carries at most 4076 bytes of plaintext and a message at most 4096 bytes. BLE frames are fixed at 20 bytes with 8 bytes of payload; frame size is not negotiated (ADR-004).
- The limits protect key material and cannot be lifted for the status channel (ADR-024).

## Decision

- **Separate tunnel keys and record kind.** An authenticated status session may open one tunnel. Its keys are derived from the session's handshake (distinct label per direction) and used in their own Noise transport state. Tunnel data travels in an own BLE message kind and does not count against the status session's record limit. The status session keeps its limits unchanged and stays usable for control operations while a tunnel is open.
- **Rekeying instead of a record limit.** Each direction rekeys (Noise `rekey`) after a fixed number of records or bytes, so the nonce space is never exhausted. Policy limits replace the hard record cap: an absolute tunnel lifetime and re-validation of the client's registration at each rekey; a revoked or blocked client loses the tunnel at the next rekey at the latest.
- **Operations.** New operations `TunnelOpen`, `TunnelClose` and a credit message for flow control, announced by a new capability bit. `TunnelOpen` carries no target: the device connects only to `sshd` on the loopback interface. The device reports "terminal unavailable" when the operator switch is off (ADR-030) or `sshd` is not reachable.
- **Flow control and bounds.** Byte credits per direction with a window of 16 frames (about 3 KiB) in flight; the measurements show why (see the validation record): BlueZ queues notifications without bound, so a bulk stream without a window delayed a keystroke echo by up to 5.6 s, and the Windows client loses notifications beyond 16 in flight; bounded queues; at most one tunnel per device (`Busy` otherwise). A tunnel ends on `TunnelClose`, abort, end of the carrying connection, idle timeout or `sshd` closing its side; each end releases the socket, keys and buffers.
- **Frame size.** The tunnel capability negotiates the frame length: up to 185 bytes (the best measured on a Raspberry Pi 4 with a Windows central: 51-63 kB/s device to client, 38-46 kB/s client to device; 20-byte frames give 3-11 kB/s), never above what the central ATT MTU allows, falling back to the 20-byte baseline (ADR-004). Small writes are coalesced for a short interval to save frames; interactive latency stays the priority over throughput.
- **Transport change.** A tunnel does not move between transports. On a loss the client opens a new tunnel over whichever transport is available and reattaches to the `tmux` session (ADR-030). Renewing the status session (ADR-024) does not close a tunnel, because the tunnel keys are independent after derivation.
- **Abuse limits.** The device limits tunnel opens per client key and time window and relies on `sshd` (`MaxAuthTries`, `LoginGraceTime`) for the rest; a tunnel that ends before SSH authentication completes counts against the client.

## Consequences

- The protocol grows by one capability, three operations and one message kind; the security profile must be reviewed for the derived keys, the rekey interval and the lifetime policy before a release.
- Throughput over Bluetooth stays low even with larger frames (35-60 kB/s measured at best) and a keystroke echo costs one round trip of about 90 ms (95th percentile 135 ms); the terminal is usable for shell work, not for bulk transfer. Window and frame size rest on measurements with one Raspberry Pi 4 and one Windows adapter (docs/validation/Terminal-Tunnel.md); the Raspberry Pi Zero W, other centrals and rekey interval, lifetime and open rate are not measured yet.
- The device holds one more socket, queue and key set while a tunnel is open; the resource budget (Architecture §10) must include it.
- Open details: concrete limits named above, the label and derivation of the tunnel keys, behaviour when the registration check at a rekey fails in the middle of a command, and whether the BLE link parameters (connection interval) are tuned for the tunnel.
