# ADR-001 – Transactional device state

**Status:** Accepted; the decision on identity storage is superseded by ADR-003  
**Date:** 2026-11-10

## Context

Registration and the closed state must become persistent together. Network changes need a journal, because a system change and the own storage do not form a shared transaction.

## Decision

The Unix device storage uses versioned binary snapshots for registration, profile settings and the operation journal. A persistent lock file serialises transactions. Changes are written to a temporary file, synchronised, renamed atomically and secured by synchronising the directory. Initialisation is allowed only in a newly created directory.

Directories get mode 0700, files 0600. Unsafe permissions, symlinks, missing data and unknown schema versions are rejected. CRC-32 detects accidental damage; protection against deliberate changes assumes a trustworthy operating-system user and a protected storage path.

The first schema version contains no private keys or Wi-Fi secrets. Key material gets a separate adapter once the security profile is defined. Profile references point to the network adapter.

## Consequences

Core and application stay independent of the storage format. Network intents are executed only after the journal commit. A failed final commit requires a reconciliation with the actual state; pending changes are not accepted automatically.

For now, at most 256 registered and blocked keys together, 256 journal entries and 1 MiB per snapshot apply. When the journal is full, new changes are rejected; results are not silently deleted. Clean-up, migration to later schemas and measured resource budgets remain open. The storage adapter is intended for Unix devices, not for the client key store.
