# ADR-030 – Interactive terminal over SSH

**Status:** Accepted. Implemented over IP (client terminal page, `zorvinkel-terminal`); the Bluetooth tunnel is ADR-031.

## Context

Experts should open an interactive terminal on the device from the client, without switching to another application. The device service runs unprivileged with an empty capability set and `NoNewPrivileges` (ADR-016); it can neither check operating-system passwords nor start processes under another user. A terminal session must also survive interruptions: a change between Bluetooth and IP, or a short dropout (ADR-024). An SSH connection is bound to one transport stream and cannot move to another.

## Decision

- **SSH, not an own PTY service.** The terminal is a real SSH session to the device's `sshd`. Authentication stays with the operating system: password, or an SSH key deposited once for a specific user (as with `ssh-copy-id`). A Zorvinkel registration opens the channel only; it grants no shell access (Architecture §4). The SSH key is separate from the Noise identity key.
- **The client runs SSH itself.** Choosing "SSH" on a device opens a dialog outside the terminal for user and password (or key setup). The client connects, authenticates and shows the session in an embedded terminal widget. Passwords are kept in memory for the duration of the session only and are never stored.
- **Transport.** Over IP the client connects to `sshd` directly. Over Bluetooth the device service forwards a byte stream of the encrypted channel to `localhost:22`; SSH runs inside that stream. The terminal over Bluetooth is on by default and can be disabled on the device by the operator.
- **Resumable session with `tmux`.** After login the client starts `tmux new -A -s zorvinkel` (attach, else create). When the connection breaks, the client reconnects over whichever transport is available and attaches to the same session; programs keep running. `tmux` is a package dependency of the device installation.

## Consequences

- The client starts `tmux` with `set-option mouse on` for the session `zorvinkel`, so the wheel and mouse reach `tmux` (scrolling its history, selecting panes). `tmux` owns the screen, so the terminal widget has no history of its own while `tmux` runs; Shift bypasses mouse reporting for local text selection.
- Operating-system accounts and their permissions limit what a terminal user can do; there is no Zorvinkel-specific shell restriction. Protected-unit rules (ADR-022) do not apply inside a shell.
- Keystrokes typed during an interruption can be lost. A reboot of the device ends the `tmux` session. A session survives a transport change only because the client reconnects; it is not one continuous connection.
- The Bluetooth tunnel needs fragmentation for small MTUs, flow control, abort and defined release of resources. Throughput and latency over Bluetooth are limited.
- With the terminal on by default over Bluetooth, every registered client reaches the SSH login in radio range without a network. Protection rests on the operating-system login; password logins need rate limiting.
- Open details: binding the SSH host key to the Zorvinkel device identity instead of a trust-on-first-use prompt, rate limiting of password attempts, mapping of Zorvinkel identities to permitted operating-system accounts, and the operator switch for disabling the terminal. A helper shipped with Zorvinkel could replace `tmux` later (a `dtach`-like process started under the target user).
- The transport change itself, as tried for the status channel (ADR-024), has to be settled before a terminal can rely on it.
