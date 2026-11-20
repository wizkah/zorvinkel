# Changelog

## Unreleased

- Revert / Persist follow the device (ADR-034): the restart option read (`RestartPolicyRead`) now also reports whether the active configuration differs from the one that applies after a restart (second payload byte; the one-byte form of older devices reads as "same"). The device worker keeps this up to date (with a default: the active network is not the default; otherwise: the connection is one of the service's own volatile profiles). New restart change `Revert` (code 2) switches at once back to the persisted configuration (volatile profiles dropped, saved networks and the default applied again). The client offers "Revert" and "Persist" (current configuration as default) exactly while the device reports a difference; `SetupSnapshot::restart_differs`, `SetupRequest::RestartRevert`. Verified by build, clippy, the test suite and the Linux worker tests run on a Raspberry Pi 4 (cross-built); the new device service was not installed or tried with a real Wi-Fi change, and the GUI buttons were not looked at on screen. Persist always sets a default, so a default can no longer be cleared from the GUI.

- GUI changes (ADR-033 and ADR-035): (1) Terminal: the key file field is gone; once logged in, an "Easy login" switch beside "Close terminal" keeps the session key on the device and stores it in the client data directory (on), or deletes it (off; the entry on the device goes when the terminal closes); with it on, an empty password logs in with the key. The terminal is shown only after the first successful login. (2) The secured session is always renewed automatically; the switch and the expiry warning are gone. (3) "After the device restarts" is now two buttons, "Revert" and "Persist", offered once a connection works. (4) "Use cached password" is merged into the saved-password button: the system's store first, then the one Zorvinkel remembered (request `ReadCachedCredentials` removed). (5) Device tiles show two lamps, BLE and IP, for how the latest search found the device (`DiscoveredDevice::via_ble`, `via_ip`). Verified by build, clippy and tests (new: easy login against the in-process SSH server, key store); not looked at on screen, not tried against a device.

- Fix: the GUI terminal now follows the device when its way changes. Before, a terminal started over Bluetooth kept asking for a tunnel after the device came back to the network and ended with "no tunnel over Bluetooth". The connector now picks Bluetooth or the network address anew for every attempt and abandons a pending tunnel request when the way changes. Verified by build, clippy and tests; not tried against a device.

- Packaging: `packaging/Terminal.md` documents the terminal on the device (requirements `sshd` on loopback and `tmux`, the switch file `/etc/zorvinkel/terminal-disabled`, limits); `install.sh` prints a note when `sshd` or `tmux` is missing.

- Terminal over Bluetooth in the GUI (ADR-031): the terminal page is offered when the device is connected over Bluetooth and reports the tunnel capability (otherwise it still needs a network address). The terminal's connector gets its streams from the device's worker through a broker, so tunnel requests queue behind refreshes and session renewals; a busy device is retried, a device that does not offer the terminal (switched off) ends the attempt with a message. New request `OpenTunnel`, snapshot fields `tunnel_available` and `tunnel`, and the diagnostic `zorvinkel-client --ble-ssh-test directory handle user key-file`. Run against a Raspberry Pi 4 over Bluetooth: key login and `tmux` through the tunnel, keystroke echo median 90 ms (95th percentile 110 ms, 30 samples). Verified by build, clippy and tests (73 client tests); the GUI page itself was not driven on screen. Known: the status session ends after ten minutes and the Bluetooth connection with it, so the terminal reconnects through a new tunnel about every ten minutes.

- Terminal tunnel over Bluetooth, third slice (ADR-031): GATT characteristics for tunnel frames in the status service, the device gate (`/etc/zorvinkel/terminal-disabled` switches the terminal off; `sshd` must answer on loopback; six openings per minute), forwarding of the tunnel to `sshd` in the device service, `StatusConnection::open_tunnel` as client transport and the diagnostic `zorvinkel-client --ble-tunnel-test directory handle`. Tunnels are refused over IP. Run once against a Raspberry Pi 4 over Bluetooth: the real `sshd` identification and the start of its key exchange crossed the encrypted tunnel; a switched-off device refused. Not yet: GUI integration, full SSH session and throughput through the tunnel, Raspberry Pi Zero W, idle timeout.

- Terminal tunnel over Bluetooth, second slice (ADR-031, simulation only): operation `TunnelOpen` (code 21) in the status protocol with the tunnel capability bit announced when the device has a tunnel control; the device session grants one tunnel (single slot, `Busy` otherwise, `Unavailable` when the control refuses) and ends it when the owning client loses its registration; `tunnel_io` forwards records to and from any byte stream with credit-based flow control on both ends (tests: bulk data both ways, stalled peer, policy end, forged frame, vanished transport); `SshConnector::with_dialer` runs the SSH connection over such a stream. A terminal session through the encrypted tunnel against an in-process SSH server passes. L2CAP channels were not measured because the client stack cannot open them (see the validation record). Not yet: GATT characteristic for tunnel frames, operator switch, connection to the real `sshd`, rate limit, idle timeout, tests on the Raspberry Pi.

- Terminal tunnel over Bluetooth, first slices (ADR-031, nothing connected to the services yet): (1) Measurement service `--ble-bench` on `zorvinkeld` and `zorvinkel-client` (separate GATT service, no application data) and the record `docs/validation/Terminal-Tunnel.md` with the numbers from a Raspberry Pi 4 and a Windows adapter: round trip 90 ms (95th percentile 135 ms), 35-60 kB/s per direction at best (185-byte frames), effective ATT MTU 512, BlueZ queues notifications without bound (an answer waited up to 5.6 s behind a bulk stream) and `btleplug` keeps 16 notifications per peripheral on Windows. ADR-031 now names the window (16 frames) and the frame length (up to 185 bytes) from this. (2) `zorvinkel_protocol::tunnel`: capability bit 4096, open request/response, record contents (data, credit, close) and the credit windows of both sides, with tests. (3) `security::tunnel`: key exchange `Noise_NN` inside the authenticated session, one record per frame, rekeying of both directions every 1024 records, closing on any lost, repeated or altered frame, lifetime as a parameter; 10 tests. Verified by build, clippy, tests and `cargo deny`. Not yet: operations in the status protocol, forwarding to `sshd` on the device, the client transport for `russh`, the GATT characteristics for tunnel frames, the operator switch, the measurements on the Raspberry Pi Zero W.

- Terminal, rest of the first scope: (1) "Open a terminal" is the eighth entry of the function overview (ADR-027), available once the system information holds a routable address (`SystemInfo::ssh_address`, moved from the GUI to the application crate and tested there); without one the hint says that no address was reported yet. (2) Login with a private key file (optional passphrase in the password field); the key is read only at the click. (3) Mouse and history: the client starts `tmux` with `mouse on`; programs that ask for the mouse (such as `tmux`) get wheel, click and drag as reports (X10, UTF-8 and SGR encodings), otherwise the wheel scrolls the local history; dragging selects text locally (Shift forces this even while a program takes the mouse), Ctrl+Shift+C or the platform copy shortcut copies it, a plain Ctrl+C without a selection still sends ^C. Verified by build, clippy and tests (65 client tests, 20 terminal tests, the real-device test against the Raspberry Pi 4 with the new command line); not looked at on screen. Not included yet: the Bluetooth tunnel (ADR-031), bracketed paste, middle and right mouse button.

- GUI: new "Terminal" page in the device tab (ADR-030, over IP only). It appears once the device reported a routable network address (system information) and offers a login (user, password), a trust-on-first-use question for the SSH host key (stored in `ssh_host_keys` in the client data directory; a changed key needs an explicit replace), and an embedded terminal with colours, cursor, resize, function keys and paste. A missing `tmux` (status 127) gets its own hint. The terminal runs on its own one-thread Tokio runtime per device; closing it waits up to 5 s so the session key is removed from the device. Verified by build, clippy and tests (62 client tests, including rendering of the login, host key and ending states and key translation); not looked at on screen and not driven through the GUI against a device yet. Not included: scrolling back, text selection and copying, key file login, the Bluetooth path.

- ADR-032 (proposed) and implementation: a password is used once; the client deposits the public half of an ephemeral Ed25519 session key (`restrict,pty`) in `~/.ssh/authorized_keys`, drops the password, reconnects with the key and removes the entry when the terminal ends. The private key exists in client memory only. Deposit and removal commands checked on a Raspberry Pi 4 by hand (a full password login was not run: no password is available to the test setup).

- New crate `zorvinkel-terminal` (first part of the terminal, ADR-030): SSH session over IP with password login (`russh`, `ring` backend), attaching to `tmux new-session -A -s zorvinkel`, automatic reconnect with backoff and reattach, and terminal emulation (`vt100`). Passwords live in a zeroizing buffer for the session only; host keys are checked by a caller-supplied verifier. Login is by password or by private key. Tested with unit tests and an in-process SSH server (both login kinds, wrong password, untrusted host key, resize, exit status, close) and, by an ignored test, against the `sshd` of a Raspberry Pi 4 (Debian 12): key login, pseudo-terminal and remote command work; without `tmux` the session ended with status 127; after installing `tmux` 3.3a the client attached to the session `zorvinkel`, detached cleanly (status 0) and the session stayed alive on the device. Not yet wired into the GUI. `cargo deny` licenses and advisories pass; `THIRD-PARTY-LICENSES.md` is regenerated. `Architecture.md` lists the new crate.

- ADR-031 (proposed): the terminal over Bluetooth gets its own tunnel channel with derived keys, rekeying and flow control, independent of the status session's 600 s / 1024-record limits. Decision draft only; nothing implemented.

- ADR-030: the interactive terminal will be a real SSH session run by the client, over IP or tunnelled through the Bluetooth channel (on by default, operator can disable), resumable with `tmux`. Decision only; nothing implemented.

- Documentation split for publication: Vision, Requirements, product name, plan, memory and the agent instructions (`AGENTS.md`, `CLAUDE.md`) moved to the private `zorvinkel-workspace` repository, which contains this repository as the subfolder `zorvinkel/`. `docs/` keeps architecture (with ADRs), Clean Code guidelines, logo and validation, now translated to English like all other Markdown files in the repository (`packaging/BUILD.md`, `packaging/Services.md`, `THIRD-PARTY-NOTICES.md`). ADRs still cite requirement identifiers (`FR-…`, `AC-…`) from the private Requirements.

- GUI: English and German. The language follows the operating system (English if it is neither) and can be switched in the tab bar; the choice is saved in the client data directory (`language`) and wins over the system language at the next start (the simulation saves nothing). All texts live in `gui/i18n.rs` (ADR-029); device state holds identifiers instead of rendered strings, so a switch applies at once. Uptime, sizes and decimals follow the language. Device names, service names/groups and Wi-Fi names are shown as the device reports them. New dependency `sys-locale`; `THIRD-PARTY-LICENSES.md` regenerated with `packaging/licenses.sh`. Verified by `fmt`, `clippy` and the client tests (45, including both languages and the saved choice); not looked at on screen, longer English or German texts may need layout tweaks.

- Windows client: `zorvinkel-client.exe` now carries the application icon (`crates/zorvinkel-client/assets/zorvinkel.ico`, generated from the `--export-app-icon` artwork) as an embedded resource, so Explorer, the taskbar pin and shortcuts show it. Building needs the Windows SDK resource compiler (`rc.exe`); without it the build warns and the executable keeps the default icon. Checked by a debug build on Windows 11; Explorer display itself not viewed.

- Windows client: "take over saved Wi-Fi password" reads the profile key through the WLAN API (`WlanGetProfile`). Windows normally releases the plaintext only to an elevated process for all-user profiles; otherwise the password is unavailable and has to be typed. Only the profile parser is tested; not tried against real profiles.

- Windows client: the private client identity now lives in the Windows Credential Manager (ADR-028), migrating an existing file identity without changing the key, and the Wi-Fi password cache uses it as well (split over up to four items). Clippy and the adapter tests pass on Windows 11, including ignored tests against the real Credential Manager; the GUI flow and an upgrade of a real existing installation are not tried.

- Switching the device's own Wi-Fi (or joining a Wi-Fi) over IP no longer reports a failure when the IP session ends with the network change: the client completes the task over Bluetooth by its operation identifier and shows the new status (and the QR code of the device's own Wi-Fi). The System page shows the current network (joined SSID, own Wi-Fi or not connected). Verified by build, `clippy` and tests; not yet tried on the Pi 4 in the GUI.

- The installer adds a polkit rule (`/etc/polkit-1/rules.d/50-zorvinkel.rules`) so the device service may create its Wi-Fi profiles in NetworkManager; on a Raspberry Pi 4 the device's own Wi-Fi failed with "Insufficient privileges" (ADR-018). Not yet tried on the Pi.

- Device tab: the hint "connection continues over Bluetooth" no longer stays after the session moved back to the network.

- Windows Bluetooth search showed "Zorvinkel BLE probe" instead of the device name: btleplug scans passively on Windows and so never receives the scan response that carries the name. `vendor/btleplug` (0.13.5, patched to scan actively, see `PATCHES.md`) replaces the crates.io version. Builds and clippy pass; not yet tried against a real Pi on Windows.

- Switching a device's Wi-Fi no longer reports "Anfrage fehlgeschlagen" when the change itself succeeded but the status read right after it hit the session the change had ended; the session now continues over Bluetooth and its status shows the outcome.

- Bluetooth search: when a device's name has not arrived yet (on Windows it can sit in a scan response after the first advertisement), discovery waits up to 4 s after the first hit and reads the name again before listing, instead of showing the fallback "Zorvinkel BLE probe". Clippy passes; not yet tried against a real Pi on Windows.

- Windows GUI: buttons, input fields and colours now use the Zorvinkel theme. The theme was applied to egui's default theme only, and egui switched to its own default style once Windows reported the system theme after start-up. The light theme is now fixed on all platforms (`ThemePreference::Light`, native window theme light). Checked on screen on Windows 11 (150 % scaling, light mode); dark mode not tried.

- NetworkManager is now also detected by its running process, because Raspberry Pi OS starts it without a PID file and the device service used to fall back to wpa_supplicant/dhcpcd (`DhcpcdMissing`). The PID file check stays for other systems.

- The search no longer lists an unregistered device twice: a network entry whose name equals a Bluetooth entry is hidden, so the first registration runs over Bluetooth (ADR-023). Registered devices merge by fingerprint; several contacts with one identity count as one device and the Bluetooth contact wins. Verified by a unit test and `clippy` on Windows; not yet tried against a real Pi in the GUI.

- Windows client: the desktop GUI now runs with the real BLE backend on Windows (data in `%LOCALAPPDATA%Zorvinkel`) and starts by double click without a console window (`windows_subsystem` in builds with `gui`, `security` and `ble-client`; command-line output is not shown there). Private storage relies on the user-profile ACL instead of Unix modes.

- Added `deny.toml` (license allow list, `cargo deny --all-features check licenses` passes) and `THIRD-PARTY-NOTICES.md`; reviewed the dependency licenses against GPL-3.0-or-later, including the vendored BSD-2-Clause `bluer`: no conflicts. The license check runs in CI (Linux job); the macOS app bundle and `install.sh` ship `LICENSE`, `THIRD-PARTY-NOTICES.md` and the `bluer` license. `THIRD-PARTY-LICENSES.md` holds the full license texts of all dependencies (`packaging/licenses.sh`, cargo-about) and is bundled likewise; CI regenerates it and fails on a difference.

- Aligned the requirements (0.4) and Architecture (0.6) with the implemented extras on explicit instruction: IP carries all operations with mDNS announcement, session limits/renewal and correlation proof, device power control, system information, configured services with limited actions, avatars.

- Tidied the device tab: "WLAN einrichten", "Eigenes WLAN" and "Neustart" are one page "WLAN" with a switch between joining a network and the device's own Wi-Fi (it follows the device while its own Wi-Fi is on). The restart behaviour has one control instead of three: "original settings" or "keep the current setting", the latter with a checkbox "later changes too" (on: last accepted configuration, off: fixed default); leaving the fixed default ends it. New `theme::segmented` control. Not looked at on screen beyond the unit render tests.

- Added the restart options `LastAccepted` and `DefaultProfile` to the device service for Wi-Fi and the device's own Wi-Fi on both network stacks (ADR-025): new operations `RestartPolicyRead`/`RestartPolicySet` (capability bit 2048); "current configuration as default" keeps the active Wi-Fi saved or records the own access point as default, a set default takes precedence over "keep", is applied again after a restart and after reverting an unconfirmed change. A volatile access point is now reverted after 60 s without a confirming client, like a volatile Wi-Fi change (ADR-017 said the opposite). The GUI offers the restart page on real devices.

- The device now creates its Wi-Fi name (`Zorvinkel-<hostname>`) and a random 63-character password on first start and keeps them in the protected store (`access-point-credentials/`, mode 0700/0600, ADR-026); an older plain file `access-point` is migrated and removed. Credentials are never regenerated over existing or damaged data. The client proposal stays only for devices without stored credentials.

- The GUI status area lists which functions the connected device offers (ADR-027); new `SetupError::NoActiveNetwork` for a default without an active network. Verified by unit, protocol, service and simulation tests on macOS plus `cargo check`/`clippy` of the adapters for `aarch64-unknown-linux-gnu`; nothing of this ran on a Pi or a real NetworkManager. Installations must update the device service and the client together.

- Added session renewal, IP-to-Bluetooth fallback, the one-time correlation proof and a connection-path display (ADR-024): the GUI renews the secured session 45 s before the 600 s / 1024-record limit (per-device checkbox "Sitzung automatisch erneuern", otherwise a warning with "Jetzt erneuern" and a clear "expired" error); a lost IP session continues over Bluetooth; new operations `CorrelationIssue`/`CorrelationRedeem` (capability bit 1024) tie a Bluetooth-to-IP switch to the running session; tiles, tab header and sidebar show Bluetooth or Netzwerk (IP). Fixed the `clippy::manual_checked_ops` failure in `system_page.rs`; `cargo clippy --workspace --all-targets --all-features -- -D warnings` now passes. Verified by unit, codec, service and simulation tests; the connection-path display was confirmed by the user on the Pi Zero W, the rest of the Pi run (renewal, fallback, proof) is outstanding. Installations must update the device service for the proof (without it the client stays on Bluetooth).

- Opened the IP endpoint to all operations and added device search on the local network (ADR-023): the device serves registration, status, Wi-Fi setup, access point, system information, services and power over TCP port 47470 like over Bluetooth (long-lived sessions, 15 s handshake limit, 60 s idle limit) and announces itself via mDNS/DNS-SD (`_zorvinkel._tcp`, new dependency `mdns-sd`). The client searches Bluetooth and the network together (either may be missing), prefers the network path, falls back to Bluetooth and switches a Bluetooth session to IP once reachability is confirmed. Verified by loopback tests on macOS, including a real mDNS announce-and-find run (ignored test); the Linux service and operation on the Pi are outstanding.

- Tidied the GUI tab bar: tabs and their state dots are vertically centred, and on macOS the native window buttons are centred in the bar (`gui/window_buttons.rs`, the only `unsafe` code in the client, via objc2 and raw-window-handle as macOS-only dependencies; `unsafe_code` is allowed there explicitly). Checked on macOS in the simulation GUI.

- Added starting, stopping and restarting of configured services (ADR-022): `actions = [...]` in `services.d` allows them per service, a new authenticated BLE operation `ServiceAction` (capability bit 512) queues the request, and a root helper (`zorvinkel-service.path`/`.service`, `zorvinkeld --run-service-requests`) re-checks it against the configuration and runs `systemctl`. Protected units (`bluetooth`, `dbus`, `ssh`, network stacks, `zorvinkel*`, `systemd-*`) cannot be controlled. The GUI shows the allowed buttons per service and asks before stop and restart. Installations must be updated with `install.sh`, which enables the new path unit; existing `services.d` files need `actions` to opt in.

- Added configured services to the device tab (ADR-021): the operator describes services in `/etc/zorvinkel/services.d/*.toml` (name, group, systemd unit, optional version command and web page); an authenticated read-only BLE operation `Services` (capability bit 256) reports version and state, and the GUI shows one tab per group with version, state and, for web pages, a button and a QR code. Setup and file format are in `packaging/Services.md`; `install.sh` creates the directory and an example file. New dependencies `toml` and `serde` for the device service. Verified by unit, protocol, service and simulation tests; the Pi run and the look of the new tabs are outstanding.

- Added the authenticated IP reachability test (ADR-020): the device service listens on TCP port 47470 (capability bit 128) and answers only `Authenticate` over the secured channel; the client tests the device's reported addresses in parallel, requires the pinned device identity and shows "Erreichbar und bestätigt", "Nicht erreichbar" or "Noch nicht geprüft" as "Vom Computer erreichbar". `zorvinkeld.service` now allows `AF_INET`/`AF_INET6`, so installations must be updated. Verified by loopback integration tests on macOS; the Linux service type-checks for aarch64 (`cargo check`, no link), the Pi Zero W runs the updated service and listens on port 47470 (reachable from the Mac); the full GUI check against the Pi is outstanding.

- Added system information of the device (ADR-019): authenticated read-only BLE operation `SystemInfo` (capability bit 64) reporting device model, OS, kernel, hostname, IP addresses, service version, uptime, load, free memory, free storage and, where available, SoC temperature, CPU clock and throttling flags. The GUI shows it on a new "System" page of the device tab and refreshes it with the status. Verified by unit, protocol, service and simulation tests; cross-built for ARMv6 and aarch64 Linux; a Pi run is still outstanding.

- Licensed the project under GPL-3.0-or-later (`LICENSE`, `license` in all workspace crates).

- Bird avatars (variant 1) are now shown in the GUI: tiles and the device header render them via resvg from the device name (cached texture per name). The generator lives in the separate MIT-licensed library [wizkah/zorvinkel-bird-avatars](https://github.com/wizkah/zorvinkel-bird-avatars) (public), a git dependency of `zorvinkel-client` (feature `gui`).

- Reworked the desktop GUI: the start tab "Geräte" shows every found device as a tile in a scrollable grid with its state (connected, disconnected, running action) and a connect/disconnect button, plus "Erneut suchen". Connecting opens a tab for the device; a double click only opens the tab. Every tab except "Geräte" can be closed (closing a connected device's tab disconnects it). The connection status stays in the right sidebar. Marketing texts, the tagline, the badge and the footer note were removed; the bird and wordmark moved to the right end of the tab bar, devices get generated avatars, and the progress overlay stays (shown for the visible tab). `--simulate-gui [count]` simulates up to 100 devices that connect in parallel (default 3). A real backend still holds one connection at a time.

- Added a NetworkManager adapter (ADR-018) behind the new `network-manager` feature: Wi-Fi scan, connection with rollback, volatile profiles with opt-in saving, revert of volatile changes, restart choice and the device's own Wi-Fi (NetworkManager `shared` addressing, no root helper), plus status from NetworkManager. The setup worker now drives either stack through one interface. The service starts with NetworkManager when it is running. Verified only against a simulated D-Bus interface and by cross-building; not yet tested with a real NetworkManager.

- Added switching the connected device to its own Wi-Fi (access point) over authenticated BLE (`ApRead`, `ApWrite`, `ApEnable`, `ApDisable`, capability bit 32, ADR-017). The GUI proposes `Zorvinkel-<hostname>` and a random 63-character password, both editable, and shows the QR code once the device reports the access point as active. On the wpa_supplicant/dhcpcd fallback the access point is a volatile `mode=2` profile; a root helper (`zorvinkel-ap`, needs `dnsmasq-base`) provides address and DHCP. Volatile by default, kept across restarts when the existing "keep new settings" choice is on (shown on both the Wi-Fi and the access point page). Verified by unit, protocol, service and simulation tests and a manual end-to-end test on a Pi Zero W. NetworkManager is not supported yet.

- Added shutdown and restart of the connected device: authenticated BLE operation `Power` (capability bit 16), a "Gerät" page in the GUI with confirmation, and root-owned systemd path units that perform the action for the unprivileged service (ADR-016). Verified by unit, protocol, service and simulation tests; not yet on a real Pi.

- Added a second Wi-Fi password button: "Zwischengespeichertes Passwort nutzen". After a device accepted a password, the client remembers it in one macOS keychain item (`org.zorvinkel.client.wifi`, all networks in one blob, so approval is needed at most once for the whole area). The system button is now "WLAN-Passwort vom System holen". Not yet verified against the real keychain.

- Implemented "Gespeichertes Passwort übernehmen" on macOS: on request the password of the entered network is read via CoreWLAN `CWKeychainFindWiFiPassword` (user, then system keychain; macOS asks for approval) into the password field. Other platforms and the simulation report it as unavailable. The workspace `unsafe_code` lint is `deny` instead of `forbid`; only `credentials.rs` allows the one FFI call. Not yet verified against a real keychain entry.

- Added an eye toggle to the Wi-Fi password field in the desktop GUI to show or hide the entered password.

- A volatile Wi-Fi change is reverted when it is lost for three checks in a row (5 s apart) or when no authenticated client talks to the device again within 60 s (e.g. Bluetooth became unusable because of the change). Reverting reloads the saved wpa_supplicant configuration (`RECONFIGURE`), dropping temporary profiles. A marker file keeps the pending revert across a service restart, e.g. after a BlueZ crash. Persisted connections are not affected.

- Added a GUI choice on the Wi-Fi page: keep new Wi-Fi settings after a restart (overwriting the originals) or use the original settings again. It is an authenticated BLE operation (`WifiPersistence`, capability bit 8) that switches the same flag as the local admin command and only affects connections made afterwards.

- Added opt-in persistence for the wpa_supplicant fallback (default off, local `zorvinkel-admin wifi-persistence on|off|status`): after a successful connection wpa_supplicant saves only the active owned profile; foreign profiles stay untouched.

- Added read-only startup selection of the network stack (NetworkManager preferred, wpa_supplicant/dhcpcd fallback requiring both services); an unsupported or unclear stack now starts the service without network setup instead of failing.

- Defined NetworkManager preference and wpa_supplicant/dhcpcd fallback with device capability checks.

- Added macOS client and Raspberry Pi Zero W cross-build scripts with verified ARMv6 and ARM64 D-Bus packages and automatic setup of missing cross-build tools.

- Named the macOS app Zorvinkel without suffixes; show device hostname alone and disambiguate only distinct registered identities sharing a name. Replaced per-frame SVG rasterization with reusable GPU layers and display-synchronized animation.

- Use the Linux system hostname as the BLE device-name prefix while preserving the existing device suffix and cryptographic identity.

- Made bounded Bluetooth crash recovery and dependent BLE re-registration mandatory in the Linux installer; installed and fault-tested the systemd rules on the Pi without changing identity or network configuration.

- Added BLE connection-phase error context; restored the test Pi’s failed Bluetooth service and verified repeated authenticated reconnection.

- Kept automatic GUI status refreshes silent and interactive; placed welcome-sized progress with action text in a window-centered overlay without cancellation controls or layout shifts. User actions during a refresh are deferred without being lost.

- Connected volatile Wi-Fi scanning and setup to authenticated BLE jobs and the desktop GUI, with capability gating, durable credential-free operation journaling, reconnect polling and a bounded setup worker. Verified native GUI discovery, secure connection and fresh Wi-Fi scanning against the updated Pi service.

- Added an experimental volatile wpa_supplicant network adapter for existing dhcpcd installations, bounded local socket communication, temporary WPA2 profiles, connection rollback and a manual probe. AP commands remain unsupported.

- Increased terrain line width and opacity so the progress animation remains visible at GUI size.

- Replaced the GUI busy spinner with the embedded SVG bird animation, including wingbeats and scrolling terrain.

- Added an animated SVG loading indicator with a stationary bird, faster eased single and tightly spaced double wingbeats with uneven timing, longer glide pauses a stationary tail and a tighter inward wing stroke and mostly transparent downward-scrolling terrain.

- Added the macOS bundle icon and automatic ICNS generation from the existing application artwork.

- Added macOS Keychain storage for client identities with verified, restart-safe migration preserving existing registrations and no file fallback on access failure.

- Integrated persistent secure BLE registration and status with the desktop GUI, root-verified local administration, Linux service installation and a local macOS app bundle. Added separate secure framing, a durable device catalog and awaited BlueR shutdown.

- Replaced the five-second BLE discovery snapshot with bounded event collection, deduplication and a short collection window after the first match; verified repeated discovery with the Pi’s original advertising interval.

- Added a pinned BlueR compatibility patch for BlueZ 5.50, regression tests, a confirmed-write workaround, macOS peripheral handle renewal and reconnect-safe session cleanup. Two native BLE exchanges passed; automatic discovery remains unreliable.

- Passed 90 cross-compiled workspace tests on Raspberry Pi Zero W, including Linux persistence and Noise security flows.

- Integrated the macOS title bar into the light branded header while retaining native window controls.

- Replaced the default eframe application icon with the embedded bird logo on a light blue-gray background, with rounded corners and inset margins on macOS.

- Refined the setup GUI with embedded bird and wordmark assets, a blue theme, rounded cards and clearer action styling.

- Added the Noise-protected status protocol, generated persistent identities, authenticated registration and reconnect, session revocation, security regression tests and an encrypted in-memory CLI demo.

- Added transparent bird and wordmark SVGs with PNG previews; fine contours and reconstructed color transitions still differ from the source.

- Added an optional native setup GUI with a simulated device, asynchronous requests, Wi-Fi recovery, access point QR display and restart settings.

- Added bounded BLE probe framing, fragmentation and reconnect tests, optional native client and Linux GATT probe server, and BLE feature builds in CI.

- Added setup key authorization, persistent device trust and opaque Unix identity storage.

- Accepted TOFU for first contact and specified optional client preauthorization and device key pinning.

- Added transactional registration state, targeted grants, revocation, network operation journaling, restart policies and Unix snapshot persistence.
- Added recovery, concurrency, corruption and commit-failure tests.

- Added the MVP Rust workspace, simulated discovery/status flows and three-platform build checks.

- Grouped application files and documentation under `zorvinkel/`.
- Added document maintenance rules and a development memory.
- Removed obsolete ADR-001; its transport decision is covered by the current architecture.
- Initialized a Git repository with Rust and local-file ignore rules.
