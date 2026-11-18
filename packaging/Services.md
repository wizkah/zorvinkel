# Showing services in the device tab

The device service can show further services of the device in the device tab, e.g. Raspotify with its version and state. The operator defines them as configuration files on the device; the client never sees commands or paths (ADR-021). On request the configuration also allows actions: starting, stopping and restarting services (ADR-022).

## Setup

`packaging/linux/install.sh` creates `/etc/zorvinkel/services.d/` and installs `raspotify.toml.example` there. Having the directory enables the function (capability bit 256). Create one file per service with the extension `.toml`, e.g.:

```sh
sudo cp /etc/zorvinkel/services.d/raspotify.toml.example /etc/zorvinkel/services.d/raspotify.toml
sudoedit /etc/zorvinkel/services.d/raspotify.toml
```

To control services the path unit `zorvinkel-service.path` must be active; `install.sh` sets it up. Without it the services are display only.

The service rereads the directory on every status update; restarting `zorvinkeld` is not necessary. The client learns about the function when it connects. If the directory did not exist before, disconnect once and connect again.

## File format

One file describes one service ([TOML](https://toml.io)). Unknown keys are errors so that typos stand out.

| Key | Required | Meaning |
|---|---|---|
| `name` | yes | Display name, 1–32 characters |
| `unit` | yes | systemd service, e.g. `raspotify.service`; only letters, digits and `: _ . @ -`, starts with a letter or digit, ends with `.service` |
| `group` | no | Tab in the device tab, 1–32 characters; default "Dienste". Services with the same group share a tab, tabs are sorted alphabetically |
| `version_command` | no | Command as a list, e.g. `["/usr/bin/librespot", "--version"]`; at most 8 entries of 128 bytes |
| `version_word` | no | Counts from 1: the word of the first output line that holds the version. Without it the whole first line applies |
| `actions` | no | Allowed actions as a list of `"start"`, `"stop"`, `"restart"`. Without the key the service is display only |
| `[web]` | no | The service's web interface (below) |

From `[web]`:

| Key | Required | Meaning |
|---|---|---|
| `port` | yes | 1–65535 |
| `scheme` | no | `http` (default) or `https` |
| `path` | no | Default `/`; starts with `/`, at most 64 characters, printable ASCII without spaces |
| `label` | no | Caption of the button, 1–32 characters; the client shows "Web interface" (in its language) by default |

### Example with several services in one tab

```toml
# /etc/zorvinkel/services.d/mopidy.toml
name = "Mopidy"
group = "Musik"
unit = "mopidy.service"
version_command = ["mopidy", "--version"]
version_word = 2        # output "Mopidy 3.4.2"

[web]
label = "Iris"
port = 6680
path = "/iris/"
```

Together with `raspotify.toml` (also `group = "Musik"`) both appear in the tab "Musik".

## Display

- **State** from `systemctl show`: Running, Starting, Stopped, Failed, Unknown (service not found or not readable).
- **Version** from `version_command`. The command runs without a shell as user `zorvinkel`, for at most 2 seconds; only the first 1024 bytes of stdout count. The result (a failure too) is remembered for 10 minutes. If the output is empty, longer than 32 characters or contains control characters, the version is missing.
- **Web interface:** The client builds the URL from an IP address the device reports (IPv4 preferred) and shows a button and a QR code. The button opens the page in the system browser, the QR code is meant for the phone. If the device is not yet confirmed as reachable from the computer (ADR-020), a note is shown; the page itself is not checked. If the device reports no IP address, only a note appears.

## Actions

- Each service shows exactly the buttons that `actions` allows. Stopping and restarting ask first, starting does not.
- The client sends only the action and the service name. The device service checks both against the configuration and writes a request file; `zorvinkel-service.service` (root) checks it again and calls `systemctl --no-block <action> -- <unit>`. The device service itself gets no additional privileges.
- "Accepted" does not mean "done": the client rereads the state after about a second and then with the regular refresh. Errors of the helper are in the journal: `journalctl -u zorvinkel-service.service`.
- **Protected units** never get actions, so nobody locks themselves out: `bluetooth`, `dbus`, `ssh`, `sshd`, `networking`, `NetworkManager`, `wpa_supplicant`, `dhcpcd`, `rescue` and all units that begin with `zorvinkel`, `systemd-`, `getty@` or `serial-getty@`. A file with `actions` for such a unit is skipped with a note in the journal. Display without actions is allowed.
- Service names (`name`) must be unique because requests use the name. For duplicates the first file in alphabetical order applies.
- At most eight open requests; further ones are rejected with "Busy".

## Limits and security

- Zorvinkel does not protect the web interface. Anyone who knows the address and is on the network can reach it, unless the service itself demands something. With `https` and a self-signed certificate the browser warns.
- Only registered clients receive the list and may trigger actions. The list contains name, group, version, state, allowed actions and web address (without host), but no commands or paths. Every registered client may run the allowed actions of all configured services.
- `version_command` is the only way the service starts programs from the configuration. The files should belong to `root` and be writable only by `root`; the service user cannot change them because of `ProtectSystem=strict`. Whoever may write to the directory can have commands run as user `zorvinkel`.
- At most 16 services; further files are ignored. Files over 4 KiB and files without the extension `.toml` are skipped, as are symlinks.

## Troubleshooting

Invalid files are skipped, the others stay visible. The reason is in the journal, once per change:

```sh
journalctl -u zorvinkeld.service | grep "Service configuration ignored"
```

Typical causes: a typo in a key, `unit` without `.service`, `path` without a leading `/`, a name over 32 characters. If the version is empty, try the command as user `zorvinkel`:

```sh
sudo -u zorvinkel /usr/bin/librespot --version
```
