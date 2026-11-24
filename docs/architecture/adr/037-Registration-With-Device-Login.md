# ADR-037 – Registration with the login of a device account

**Status:** Accepted. Supplements ADR-002 (first contact) and the registration rules of Architecture §4.

## Context

Registration after the first client was closed: another computer needed a registered client or the local administrator to grant its key. A user who stands in front of a device they can log in to had no way to register a new computer from that computer.

## Decision

- A client the device does not know can register with the user name and password of an operating-system account of the device. The secured session carries them (operation `RegisterWithLogin`, code 22); the answer is `Registered` or one of `LoginRejected`, `NotAdministrator`, `Throttled`, `Unavailable`, `Unsupported`. The client key of the secured session is the key that gets registered, so no second key is created or sent. Later connections authenticate with that key and need no password.
- Only administrator accounts qualify: uid 0 or a member (primary or supplementary) of `sudo`, `wheel`, `admin` or `root`. The check follows the password, so a wrong password does not reveal which accounts exist or are administrators. A registered client controls the network, restarts and services, which is why a plain account is not enough.
- The password is checked by `sshd` of the device itself, because the service runs unprivileged and cannot ask PAM. The service starts the `ssh` program towards `127.0.0.1:22` with password and keyboard-interactive authentication only and an empty environment except one variable carrying the password, which the program's `SSH_ASKPASS` helper (the service binary, which prints that variable and exits) hands over. Success is the exit status 0 of `exit 0` on the device. A refusal that offers no password method means password login is off: that is "unavailable", not "wrong". No new privilege and no new service is needed.
- Wrong logins are limited device wide: five within 300 seconds, then logins are refused until the oldest failure leaves the window. Only this registration path is affected; registered clients keep working (NFR-SEC-005). A refused login ends the secured session, so every attempt costs a new handshake.
- Registration stays a decision of the device registry: the login authorises exactly the key of this session (as an authorised local administrator would), closes registration and registers nothing for a client that was blocked before (`Revoked` stays refused). The open first registration of the default mode stays as it is; the login is an additional way, also available while the device is unclaimed.
- The client offers the login after a refusal because registration is closed. The password is taken out of the form when sent, wiped after use and never stored.

## Consequences

- At the first contact the device identity is accepted by TOFU (ADR-002). A login at that moment hands the password to whatever device answered, so an impersonator in range at the very first contact could read an administrator password. With an expected device key deposited in the client (ADR-002) the check precedes the login, and the risk does not exist. The login page says that the password is sent once; this limit is documented in the architecture.
- A wrong guess costs the attacker the shared limit, but anyone in range can also use up the limit and keep a legitimate user from registering for a few minutes (a denial of this one path, not of the device).
- The device needs the `ssh` client and an `sshd` with password login. Devices without it do not offer the operation and answer `Unsupported`; an older service answers like an unknown client.
- The password appears in the environment of one short-lived `ssh` process (readable by the service user and root) and in the service's memory for the length of the check; the secure channel and the helper keep no copy afterwards. Operating-system accounts with `AllowUsers` rules or locked accounts are refused by `sshd` and show as a wrong login.
- Administrator means group membership, not a check of the sudoers rules: an account with a custom sudoers entry outside these groups is refused.
