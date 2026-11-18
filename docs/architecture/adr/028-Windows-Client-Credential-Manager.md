# ADR-028 – Windows client identity in the credential store

**Status:** Accepted  
**Date:** 2026-11-18  
**Supersedes:** the Windows file storage of the private client identity from ADR-009 (analogous to ADR-010 for macOS)

## Context

As on macOS, the private client key should live in the operating-system key store without losing existing registrations.

## Decision

The Windows client uses the Windows credential store (Credential Manager, generic credentials of the signed-in user, persistence `LOCAL_MACHINE`, no roaming) through `windows-sys` 0.61. Access lives in a single module with `unsafe` calls.

The identity follows the flow of ADR-010: target name `org.zorvinkel.client.identity/<public key as hex>`; entries are only created or read, never overwritten (the system function replaces silently, so a check is made beforehand). The identity file contains only the versioned public reference; migration, reading back and error behaviour are unchanged. There is no fallback to private file storage.

The client cache for Wi-Fi passwords lives under `org.zorvinkel.client.wifi/networks/<n>`. An entry holds at most 2560 bytes; the data is therefore split into up to four entries, each starting with the number of parts. A deviating part count or a missing part makes the cache unusable; it is not guessed or overwritten.

## Consequences

Access requires no operating-system prompt. Other processes of the same user can read the entries; the protection corresponds to the user account, not to an application binding. An abort in the middle of writing the cache can make it unusable until the entries are deleted.
Saved Wi-Fi passwords of the system are read through the WLAN API (`WlanGetProfile` with plain-text key) on explicit request, only for the profile with the name of the SSID. Windows releases the plain text for "all users" profiles only to an elevated process; otherwise the password is unavailable and the user enters it themselves.

API: [Credential Manager](https://learn.microsoft.com/windows/win32/api/wincred/).
