# ADR-032 – Ephemeral SSH session key for the terminal

**Status:** Proposed. Supplements ADR-030 (authentication and reconnect).

## Context

A terminal that reconnects after an interruption (ADR-030) needs credentials again. Keeping the operating-system password for that purpose is unacceptable: the SSH library copies the password into ordinary strings that cannot be cleared reliably, and the password would stay in client memory for the whole session.

## Decision

- The password is used once. After a successful password login the client generates an Ed25519 key pair in memory, appends the **public** key to `~/.ssh/authorized_keys` of that user on the device and drops the password. Reconnects authenticate with the key. The private key is never written to disk and is cleared when the terminal closes.
- The entry carries the options `restrict,pty` (no forwarding, no agent, no commands beyond a shell with a pseudo-terminal request) and a comment `zorvinkel-session-<unix time>-<random hex>` that identifies it.
- When the terminal ends (closed, program finished, failed for good, or the client object is dropped) the client connects once more with the key and deletes exactly its own entry by that comment.
- If the client dies before it can remove the entry, a harmless line remains: its private key existed only in the memory of the dead process. Later runs may remove old `zorvinkel-session-*` lines.
- If key login fails on a reconnect (entry removed, home directory changed), the client reports failed authentication and asks for the login again.

## Consequences

- The device is changed: `authorized_keys` is created or extended with the user's permissions. Operators who forbid key login or make `authorized_keys` read-only cannot use the terminal; the client then reports that the key could not be deposited and does not fall back to keeping the password.
- The password still passes through the SSH library once during the first login; this short exposure is inherent in password login. Users who want to avoid it use their own private key (ADR-030).
- The window between a crash and the cleanup leaves clutter, not a usable credential.
- Open: whether `sshd` options such as an expiry time are available on all supported device versions, and removing stale entries automatically.
