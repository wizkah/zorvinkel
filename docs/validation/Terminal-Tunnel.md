# Bluetooth link measurements for the terminal tunnel

**Date:** 2026-11-20

## Environment and scope

Device: Raspberry Pi 4 Model B Rev 1.2, Raspberry Pi OS 12 (Bookworm), kernel 6.12.47, BlueZ 5.66, Cypress/Infineon controller (HCI/LMP 5.0). Client: Windows 11 (10.0.26300), Intel Wireless Bluetooth adapter, `btleplug` 0.13.5 with the patches in `vendor/btleplug`. The distance was not measured (same room, a few metres). The Raspberry Pi Zero W (BlueZ 5.50), which ADR-007 targets, was **not** measured; its numbers will differ.

The measurement service is part of the code: `zorvinkeld --ble-bench` on the device (own service `…4300`, no application data) and `zorvinkel-client --ble-bench` on the client. The device side ran as a transient service (`systemd-run --unit=zorvinkel-bench`) next to the regular `zorvinkeld.service` and was removed afterwards. Each table below comes from one or two complete runs of the release build of the client (`cargo build --release`); a debug build gave the same picture with more variation. No repeated runs over hours, no moving obstacles, no interference test. The numbers describe this pair of adapters, not the radio in general.

## Results

| Property | Result |
|---|---|
| ATT MTU | The client reports 517; writes of up to 509 bytes arrive, 511 and 512 do not (effective MTU 512). Notifications of 512 bytes are delivered. |
| Round trip, 20-byte write, echo as notification | Median 90 ms, 95th percentile 135 ms, maximum about 140–180 ms. The connection interval chosen by the Windows adapter is therefore about 45 ms; a round trip takes two intervals. |
| Round trip, 509-byte frames | Median 95–107 ms, 95th percentile 175–225 ms, maximum 265–315 ms. |
| Write with or without response | The same round trip (90 ms median). |
| Throughput device to client | 20-byte frames 11 kB/s, 64 bytes 27–31 kB/s, 128 bytes 41–49 kB/s, 185 bytes **51–63 kB/s** (best), 244–512 bytes 33–48 kB/s. |
| Throughput client to device | 20-byte frames 3 kB/s, 64 bytes 21–27 kB/s, 128 bytes 35–42 kB/s, 185 bytes 38–46 kB/s, 244–509 bytes 31–42 kB/s. |
| Loss, 200 kB per run | Zero in both directions at 128 bytes and larger; at most 2–3 frames of 10 000 at 20 and 64 bytes and one frame at 400 bytes (one run each). |
| Burst of up to 256 frames without flow control | No loss at any frame size (one run lost 12 of 128 frames of 20 bytes). |
| Interactive delay behind a bulk stream | Without flow control the device queued about 200 kB of 185-byte notifications; the answer to a 20-byte request waited **up to 5.6 s** (median 76 ms for the early ones). |

## Findings

- **No window, no interactivity.** BlueZ buffers notifications of the acquired notification socket without bound. A terminal that streams output (`cat` of a large file) therefore delays every later answer, including keystroke echo, by the whole backlog. A byte window in the tunnel itself is required, not optional.
- **Windows drops notifications beyond 16 in flight.** `btleplug` keeps notifications of a peripheral in a broadcast channel of 16 entries (`vendor/btleplug/src/winrtble/peripheral.rs`); a consumer that falls behind loses the oldest ones. The runs show no loss because the consumer kept up, but a tunnel must not rely on that: the window has to stay below 16 frames, or the capacity has to be raised in the vendored copy.
- **Frame size.** Below 128 bytes the link is limited by the number of packets per connection event (about 11 kB/s at 20 bytes, 3 kB/s for writes). Beyond 185 bytes the numbers do not improve and vary more. The existing fixed 20-byte frames (ADR-004) would give 3 to 11 kB/s and are unsuitable for the tunnel.
- **Latency for typing.** A keystroke and its echo cost one round trip: about 90 ms, up to 135–180 ms. That is usable for shell work and sluggish for full-screen editors.
- **Throughput.** About 35–60 kB/s per direction at best; copying files or large outputs takes minutes per few MB.

## Limits of these findings

- Raspberry Pi Zero W, macOS and Linux clients, other Windows adapters, longer distance, walls and radio interference were not measured. A different central chooses a different connection interval; the numbers scale with it.
- One run per condition, measured in one afternoon. No confidence intervals.
- The client measured in release mode; time spent in `btleplug` and the Rust runtime is part of the numbers.
- The measurement code counts frames, not application data; encryption, the 16-byte authentication tag and the tunnel header are not included.

## Repeating the measurement

On the device (needs the `ble-server` build; run beside the regular service):

```sh
sudo systemd-run --unit=zorvinkel-bench --collect /path/to/zorvinkeld --ble-bench
sudo systemctl stop zorvinkel-bench
```

On the client (needs the `ble-client` feature; discovery can need several tries):

```sh
cargo run --release -p zorvinkel-client --features ble-client,security -- --ble-bench
```

The run takes about eight minutes. After an aborted run the client may still hold the connection; the device then does not advertise until it is closed.

## L2CAP connection-oriented channels instead of GATT (not measured)

**Date:** 2026-11-20. Question: would an LE credit-based channel (L2CAP CoC) beat GATT notifications for the tunnel? It was **not measured**, because the client stack cannot open one:

- `btleplug` 0.13.5 (with the local patches) has no L2CAP API on any platform (no match in `vendor/btleplug`).
- The Windows bindings in use (`windows` 0.62.2) contain L2CAP only as classic Bluetooth constants and structures (`Win32::Devices::Bluetooth`, `BTHPROTO_L2CAP`); the Bluetooth LE API (`Windows.Devices.Bluetooth`) has no channel type. No other Windows interface for LE channels was looked for beyond these bindings.
- The device side could do it: `bluer` (vendored) offers L2CAP sockets on Linux. macOS has `CBL2CAPChannel`; neither was tried.

A measurement would need new platform code for the Windows client first, with an unclear outcome. The GATT numbers above stay the basis of ADR-031. Throughput of a CoC is not known to be better: both paths share the connection interval chosen by the central, which dominated the round trip (about 90 ms).

## Tunnel end to end over Bluetooth (first real run)

**Date:** 2026-11-20. Same pair as above (Raspberry Pi 4, Windows 11 with an Intel adapter), regular `zorvinkeld.service` rebuilt with the tunnel, client `zorvinkel-client --ble-tunnel-test <directory> <handle>` over Bluetooth (the network path is skipped on purpose).

- The tunnel opened through the authenticated status session; the device connected to its `sshd` on loopback and the client read the real identification line (`SSH-2.0-OpenSSH_9.2p1 …`) through the encrypted tunnel 31-38 ms after the open, then sent its own line and received 512 bytes of the key exchange within 134-135 ms. Two runs, same picture. The device logged `Tunnel ended: Peer(Normal)` after the client closed.
- With `/etc/zorvinkel/terminal-disabled` present the device refused with `Unavailable`; removing the file allowed the next run.
- Not measured: throughput and latency of an interactive SSH session through the tunnel (only about 0.5 KB were transferred), a full SSH login, rekeying on a real link, several clients, the Raspberry Pi Zero W, macOS and Linux centrals, a loss of the radio link during a tunnel. After a failed run the Windows client sometimes needed one more try to connect (known behaviour with a lingering connection).

## SSH session through the tunnel (real run)

**Date:** 2026-11-20. Same pair; `zorvinkel-client --ble-ssh-test <directory> <handle> <user> <key-file>` opens the tunnel over Bluetooth, logs in to the device's `sshd` with a private key through `SshConnector::with_dialer` and attaches to the `tmux` session. The tool accepts any host key.

- The SSH handshake and login took 0.5-0.6 s from the tunnel being open (two runs).
- 30 single keystrokes, each sent after the screen had been quiet for 400 ms and timed until the echo reached the client: **median 90 ms, 95th percentile 110 ms, maximum 135 ms**. That is one connection-event round trip as measured before; the encryption, the credit window and `tmux` add nothing visible.
- Not measured: bulk output through the tunnel (`tmux` redraws only the screen, so the test cannot produce a stream), a long session, rekeying on a real link (1024 records per direction), the Raspberry Pi Zero W, other centrals. One earlier attempt right after the previous run failed with "Not connected" (the lingering-connection behaviour noted above).
