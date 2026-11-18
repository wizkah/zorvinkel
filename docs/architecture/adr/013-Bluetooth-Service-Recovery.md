# ADR-013 – Local Bluetooth recovery

**Status:** Accepted  
**Date:** 2026-11-12

## Context

The Bluetooth service of the test Pi crashed with a memory error. Zorvinkel stayed active but could no longer accept BLE connections. Recovery must work without the desktop client and without existing network access.

## Decision

On every installation and update, the Linux installer sets up recovery as a binding part of the installation:

- A named systemd drop-in for Bluetooth sets `Restart=on-failure`, a three-second wait and at most five starts within 60 seconds.
- `zorvinkeld.service` uses the same restart limits and ties its runtime to Bluetooth with `BindsTo=bluetooth.service` and `After=bluetooth.service`.
- With `Wants=zorvinkeld.service`, the Bluetooth drop-in pulls the device service back in when Bluetooth restarts. GATT service and advertising are thereby registered again.
- Installation and update check the existing Bluetooth unit, install the rules and reload systemd. Existing identities and registrations are preserved.

## Consequences

The distribution unit is not replaced. The drop-in changes the recovery of the shared Bluetooth service; a restart interrupts its connections. A deliberate manual stop is not undone automatically.

After the start limit is reached, administrative troubleshooting remains necessary. Hanging processes without a crash, faulty radio hardware and the cause of the BlueZ memory error are not fixed by this. An existing client connection is interrupted and has to be set up again.
