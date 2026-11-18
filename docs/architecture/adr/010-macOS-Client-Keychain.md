# ADR-010 – macOS client identity in the keychain

**Status:** Accepted  
**Date:** 2026-11-11  
**Supersedes:** the macOS file storage of the private client identity from ADR-009

## Context

The client identity should live in the operating-system key store where possible. Existing registrations must not be lost in the switch.

## Decision

The macOS client uses the default file keychain of macOS through `security-framework` 3.7.0. The service name is `org.zorvinkel.client.identity`; the account is derived from the public client key. Entries are only created or read, never overwritten. The library uses the macOS keychain API for this; no cloud synchronisation is requested.

The protected identity file then contains only a versioned public reference. An existing file identity is checked under a file lock, stored in the keychain and read back completely. Only when the key material is identical is the reference written atomically. An abort before the commit leaves the previous file usable for the same migration attempt. Matching keychain entries that already exist are reused.

Missing, inaccessible, damaged or deviating keys lead to an error. There is neither a new replacement identity nor a fallback to private file storage. New installations also write only the public reference to the file system. Device binding and registration stay unchanged.

## Consequences

For the Noise handshake the private key is loaded temporarily into an opaque memory handle. Own temporary byte buffers are erased on release. The active identity file no longer contains a private key; existing backups and disk blocks that have already been released are not cleaned up retroactively.

Keychain access is subject to macOS permissions and the application signature. A locked keychain or denied access prevents login. A changed signature may require renewed operating-system approval. The local app remains ad-hoc signed; a notarised release is not accepted.

Linux device service, Windows persistence and the Linux client storage are not changed by this. The other decisions of ADR-009 continue to apply.

API: [security-framework](https://docs.rs/security-framework/3.7.0/security_framework/os/macos/keychain/struct.SecKeychain.html).
