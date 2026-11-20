# ADR-033 – Easy login for the terminal

**Status:** Proposed. Supplements ADR-032 (the session key may stay) and replaces the private key file option of ADR-030.

## Context

ADR-032 deposits a session key for the length of one terminal and removes it afterwards, so every opening of the terminal needs the password again. Logging in with a key file of the user's own was the alternative; it asks the user to manage keys and paths by hand.

## Decision

- The terminal page offers no key file. The login is user and password.
- The terminal itself is shown only after the first successful login; until then the page shows the connection state only.
- Once logged in, a switch "Easy login" sits beside "Close terminal".
  - **On:** the session key of ADR-032 stays in `authorized_keys` of the device when the terminal ends, and its private half is written to the client's data directory (`easy_login/`, one file per device, user name in the first line, key in OpenSSH format without passphrase; owner-only file mode where the platform has one).
  - **Off:** the stored private key is deleted at once. The entry on the device is removed when the terminal ends, because a reconnect inside the running terminal still needs the key.
- With an easy login stored, the login page fills in the user and an empty password logs in with the stored key; the switch then starts out on. A typed password always logs in with the password.
- If the device refuses the stored key (entry removed, home directory changed), the client deletes the stored key, switches easy login off and asks for the password.
- The entry keeps the options `restrict,pty` and the comment of ADR-032.

## Consequences

- The private key lies unencrypted in the client's data directory. Whoever can read that directory can open a terminal on the devices with easy login on, as the user, until the switch is turned off or the line is removed from `authorized_keys`. On Windows the protection is the user profile's access rights; there are no file modes.
- Turning the switch off while a client is not running cannot remove the entry on the device; only the next run with the key (or the user on the device) can. Not solved: removing stale entries automatically (see ADR-032).
- The key file option (and its passphrase handling in the password field) is gone from the client; the library function for private-key credentials stays for tests and the diagnostic.
