# ADR-003 – Identity and trust storage

**Status:** Accepted  
**Date:** 2026-11-10  
**Supersedes:** the decision on identity storage in ADR-001

## Context

Key storage and device binding must be verifiable independently of the later choice of the security adapter. Private identities do not belong in the general device state.

## Decision

A separate Unix identity store takes over a key pair verified by the security adapter and returns an opaque handle and the public key. It offers no export of private keys. Generating and cryptographically checking the keys remains the task of the security adapter.

A separate trust store holds the expected public device key for each stored device. TOFU and depositing during setup use the same atomic binding rule: an existing key is not silently replaced.

All three stores use the same file lock and atomic write logic, but separate directories and versioned file formats. Opening creates neither identities nor empty trust states. First initialisation requires a new directory.

## Consequences

Key material is protected by directory permissions 0700 and file permissions 0600, not additionally encrypted on disk. The operating-system user and the storage path are part of the trust boundary. Missing or damaged data lead to an error instead of replacing an identity or binding again through TOFU.

The file storage serves device integration and the Unix checks. Client key stores on macOS, Windows and Linux and the cryptographic proof of possession remain part of the platform and security integration. The other decisions of ADR-001 continue to apply.
