# Native secured status

**Date:** 2026-11-11

## Environment

macOS ARM64; Raspberry Pi Zero W Rev 1.1, ARMv6, Raspbian 10, BlueZ 5.50 and existing wpa_supplicant. No NetworkManager. The service changes no network profiles.

## Operation

Build the device service with the features `security,ble-server`. On the Pi as root:

```sh
packaging/linux/install.sh /path/to/the/programs
```

The program directory contains `zorvinkeld` and `zorvinkel-admin`. The first time, the installation creates the protected storage `/var/lib/zorvinkel` and enables the system service. Existing storage must be fully readable; updates create no replacement identity.

macOS client:

```sh
cargo run --locked -p zorvinkel-client --features gui,security,ble-client
```

The client uses `~/Library/Application Support/Zorvinkel`. A local app package is created with `packaging/macos/package.sh client-binary output.app`. It is ad-hoc signed, not notarised. The package contains an ICNS app icon from the existing window icon. Linux uses `XDG_DATA_HOME/Zorvinkel`, alternatively `~/.local/share/Zorvinkel`.

The interface offers real search, registration, connection, disconnecting and regular status queries. Wi-Fi changes, device Wi-Fi and restart configuration are not yet available. A Wi-Fi login does not confirm reachability over IP.

Targeted local administration on the Pi, each as root:

```sh
zorvinkel-admin /run/zorvinkel/admin.sock public-key
zorvinkel-admin /run/zorvinkel/admin.sock grant CLIENT_PUBLIC_KEY_HEX
zorvinkel-admin /run/zorvinkel/admin.sock authorize CLIENT_PUBLIC_KEY_HEX
zorvinkel-admin /run/zorvinkel/admin.sock revoke CLIENT_PUBLIC_KEY_HEX
```

`grant` applies for five minutes to exactly one client key. `authorize` deposits a client key immediately. `revoke` blocks the client permanently and ends its security sessions.

Optional trust anchor at the client:

```sh
zorvinkel-client --client-key CLIENT_DIRECTORY
zorvinkel-client --trust-device CLIENT_DIRECTORY DEVICE_HANDLE DEVICE_PUBLIC_KEY_HEX
zorvinkel-client --device-status CLIENT_DIRECTORY DEVICE_HANDLE
```

The device key must be known over an already trustworthy path. The search handle assigns the record; it proves no identity. Deviating or missing existing bindings are not replaced.

## macOS keychain

The private client identity lives in the macOS default keychain. Existing file identities are taken over on opening under a file lock. Only after an identical read-back is the active file replaced by a public reference. Missing or denied entries lead to an error; there is no replacement identity and no private file storage as a fallback.

The existing client identity was taken over; the public key stayed the same. Afterwards, two secured connections to the existing Pi succeed with the updated app. A separate native test checks migration, reopening and the error after removing its own test entry. It changes no existing keychain entries.

## Evidence

- macOS: 102 workspace tests with all features passed; an additional native keychain test passed separately; build and Clippy passed without warnings.
- Windows: cross type check with all features passed; no execution.
- ARMv6: device service and administration program cross-built optimised; Clippy for workspace and tests passed without warnings.
- Five new integration tests on the Zero passed, including persistent BLE registration flow, deleted device binding and local revocation of an active security session. The transport of these tests is simulated.
- Real radio connection: registration, repeated authentication, capabilities and Wi-Fi status successful; two fresh connections with the same stored identity.
- GUI: search, secure connection, actual Wi-Fi status, separate IP reachability and disconnecting checked visually. The app icon was confirmed visually in Finder on the updated package; plist and signature check passed.
- Linux: service runs without root, with the supplementary group `netdev`; a call of the local socket under the service user is rejected.
- Update and regular service restart preserve the public device key. After an explicit BLE deregistration, the service starts again with advertising.
- An unknown client is rejected over the secured channel. After a targeted administrative grant and deposit of the device key, registration and two fresh connections succeed. After revocation the same test client is rejected again; the authorised client then has access again without a service restart.
- Single measurement on the optimised service: 4956 KiB RSS, two threads, 0.6 % average CPU since service start after about two minutes. No load evidence.

## Restart and visibility

Advertising stays explicitly visible without limit; after 197 seconds of unchanged service operation, search and two new connections succeed. After a forced process abort and a complete device restart, the service starts automatically with the same device identity. The permanently stored client then reads the actual Wi-Fi status over two fresh connections each time.

## Limits

During integration, hanging or empty search runs occurred with BlueZ 5.50. Deregistration, rejection follow-up and visibility duration were adjusted. The final individual runs pass; long-term reliability and parallel native client processes are not accepted.

No independent security review, no load/radio interference acceptance and no reliable resource budgets. Linux client identities still live in protected Unix files; Linux/Windows key stores and Windows persistence are missing. Locked and denied keychain access is checked in simulation; the user keychain was not locked for this. Old backups and earlier disk blocks are not cleaned up. Changes of the application signature may require a renewed macOS approval. Supported minimum operating-system versions are not defined. Unsupported SSID encodings and missing status rights produce an error. No Wi-Fi scan, no network change and no IP management service.
