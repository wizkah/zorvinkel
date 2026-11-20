# Terminal on the device

The client's terminal is an SSH session to the device's own `sshd`. Over the network it connects directly. Over Bluetooth the device service forwards an encrypted tunnel to `sshd` on the loopback interface (ADR-030, ADR-031). Registering with Zorvinkel does not grant a shell: the operating-system account still logs in with a password or key.

## Requirements on the device

- `sshd` listening on `127.0.0.1:22`. The service checks this before it opens a tunnel; without it the client is told that the terminal is unavailable.
- `tmux`. The client attaches to the session `zorvinkel`; without `tmux` the terminal ends with exit status 127 and the client says so. Install it with `apt install tmux`.

`install.sh` prints a note when one of them is missing; it installs neither.

## Switching the Bluetooth terminal off

The terminal over Bluetooth is **on by default**. Create this file to switch it off:

```sh
sudo touch /etc/zorvinkel/terminal-disabled
```

Remove it to switch the terminal on again:

```sh
sudo rm /etc/zorvinkel/terminal-disabled
```

The file is checked at every request, so no restart is needed. A tunnel that is already open keeps running until it ends. With the file present the device answers a tunnel request with "unavailable" (the client then reports that the device does not offer the terminal); it still announces the capability. The switch only concerns the tunnel over Bluetooth; it does not touch `sshd` itself, which stays reachable over the network as configured in the operating system. To stop that, disable `ssh.service` as usual.

## Limits

- One tunnel per device at a time; a second request is answered "busy".
- At most six tunnel openings per minute. A missing `sshd` does not count.
- A tunnel ends after eight hours (provisional), when the client's registration is withdrawn, or when the Bluetooth connection ends. The status session over Bluetooth ends after about ten minutes and the connection with it, so the terminal reconnects through a new tunnel and reattaches to `tmux` about that often.
- Throughput over Bluetooth is a few tens of kB/s and a keystroke echo takes about 90 ms (docs/validation/Terminal-Tunnel.md).
