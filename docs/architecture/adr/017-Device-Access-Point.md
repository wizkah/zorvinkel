# ADR-017 – Device Wi-Fi (access point)

**Status:** Accepted. Partly superseded by [ADR-025](025-Restart-Options.md) (restart choice, 60-second rollback) and [ADR-026](026-Access-Point-Credentials.md) (credentials, storage).

## Context

The device should provide its own Wi-Fi whose name and password the user can see and change beforehand. The device service runs without rights; addressing and DHCP need root. The NetworkManager adapter follows in ADR-018.

## Decision

- Authenticated BLE operations `ApRead`, `ApWrite`, `ApEnable`, `ApDisable` and capability 32. Switching on and off are network jobs and are followed through `WifiPoll` like Wi-Fi jobs, are journalled and exclude each other with other network jobs.
- *(Superseded by ADR-026: the device generates the credentials itself.)* For a device without saved credentials, the client suggests `Zorvinkel-<hostname>` and a random password of 63 characters `[A-Za-z0-9]` (about 375 bits, more than the 256 bits of the WPA2 key). Both are editable. The credentials are saved on the device only when switching on. Without a randomness source there is no suggestion.
- The QR code appears only when the device reports the AP as active, and shows the data supplied by the device.
- Fallback (wpa_supplicant/dhcpcd): wpa_supplicant starts the AP as a volatile profile (`mode=2`, WPA2, CCMP, channel 6). Address `192.168.4.1/24` and DHCP (dnsmasq) are provided by a root helper which the service triggers through request files in `/run/zorvinkel` (`zorvinkel-ap.path`/`.service`, as with ADR-016). The function is offered only if the installer has enabled the helper.
- Volatile is the default: after a restart the saved Wi-Fi configuration applies. The existing "keep" choice (`WifiPersistence`) also applies to the AP: the service then switches the AP on again after start; `wpa_supplicant.conf` stays unchanged. A Wi-Fi connection or switching off cancels this re-enabling.
- *(Superseded by ADR-025.)* A volatile AP is not ended by the 60-second rollback of the Wi-Fi switch, because other devices should be able to use it without an open client.
- Under NetworkManager ADR-018 applies: NetworkManager handles address and DHCP, the root helper is omitted.

## Consequences

Credentials were in plain text in `/var/lib/zorvinkel/access-point` (mode 0600); since ADR-026 they are in protected storage. A fixed subnet can collide with the location. The behaviour of dhcpcd, dnsmasq and the wpa_supplicant AP on the Pi Zero W has so far been checked only in simulation.
