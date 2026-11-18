# ADR-026 – Device Wi-Fi credentials: generation in the service and protected storage

**Status:** Accepted. Replaces in ADR-017 the statements on the client suggestion and the plain-text file.

## Context

The requirements call for the device Wi-Fi to have a distinguishable name and a device-specific password without manual pre-configuration, retrievable by registered clients and changeable. So far the client suggested both, and a device that had never been switched on had none. The credentials were in a plain-text file (0600); the security requirements and Architecture §6 left protection in the fallback open.

## Decision

- The device service generates the credentials on first start, as soon as the device can offer a device Wi-Fi: name `Zorvinkel-<hostname>` (at most 32 bytes), password of 63 characters `[A-Za-z0-9]` from the system's randomness source. Without a randomness source no credentials are created. Existing credentials are never replaced. The device Wi-Fi stays switched off in the process.
- Registered clients read and change them as before (`ApRead`, `ApWrite`). The client suggestion remains only for devices that report no saved credentials.
- Storage in the protected storage like identity and trust (ADR-003): `/var/lib/zorvinkel/access-point-credentials/` (directory 0700, file 0600, shared lock, atomic replacement, check of permissions and file type, versioned format `zorvinkel-access-point-v1`). Opening creates nothing; damaged or too open data is an error, not a regeneration.
- A plain-text file of earlier versions (`access-point`) is taken over and removed on first read. If it is damaged, it is left in place for inspection and the service offers nothing.
- The content is not additionally encrypted. On a device without a hardware key, a key would lie on the same disk; the trust boundary remains the operating-system user and the path, as with the identity keys.

## Consequences

- Whoever controls the disk or the service user can read the password; the same applies to the identity of the device.
- The Wi-Fi password of the main network still lives exclusively in the configuration of the network stack (wpa_supplicant or NetworkManager).
- The service user must be allowed to create a directory in `/var/lib/zorvinkel` (given).
- Not checked on the Pi are the takeover of the old file and the generation on first start.
