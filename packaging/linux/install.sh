#!/bin/sh
set -eu
[ "$(id -u)" = 0 ] || { echo 'Run as root.' >&2; exit 1; }
[ "$#" = 1 ] || { echo 'Usage: install.sh directory-with-zorvinkeld-and-zorvinkel-admin' >&2; exit 1; }
source_dir=$(CDPATH= cd -- "$1" && pwd)
package_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
[ -x "$source_dir/zorvinkeld" ] && [ -x "$source_dir/zorvinkel-admin" ]
systemctl cat bluetooth.service >/dev/null
getent group netdev >/dev/null
if ! getent passwd zorvinkel >/dev/null; then
    useradd --system --user-group --no-create-home --home-dir /nonexistent --shell /usr/sbin/nologin zorvinkel
fi
# Stop the old executable before replacement; the persistent installation is never recreated.
if systemctl cat zorvinkeld.service >/dev/null 2>&1; then
    systemctl stop zorvinkeld.service
fi
install -d -m 0755 /usr/local/libexec
install -m 0755 "$source_dir/zorvinkeld" /usr/local/libexec/zorvinkeld
install -m 0755 "$source_dir/zorvinkel-admin" /usr/local/sbin/zorvinkel-admin
if [ ! -e /var/lib/zorvinkel ] && [ ! -L /var/lib/zorvinkel ]; then
    /usr/local/libexec/zorvinkeld --initialize /var/lib/zorvinkel
    chown -R zorvinkel:zorvinkel /var/lib/zorvinkel
fi
runuser -u zorvinkel -- /usr/local/libexec/zorvinkeld --check-storage /var/lib/zorvinkel
install -m 0644 "$package_dir/zorvinkel-dbus.conf" /etc/dbus-1/system.d/zorvinkel.conf
# NetworkManager asks polkit; without this rule the service may not create its Wi-Fi profiles.
install -d -m 0755 /etc/polkit-1/rules.d
install -m 0644 "$package_dir/zorvinkel-polkit.rules" /etc/polkit-1/rules.d/50-zorvinkel.rules
install -m 0644 "$package_dir/zorvinkeld.service" /etc/systemd/system/zorvinkeld.service
install -m 0644 "$package_dir/zorvinkel-power@.path" /etc/systemd/system/zorvinkel-power@.path
install -m 0644 "$package_dir/zorvinkel-power@.service" /etc/systemd/system/zorvinkel-power@.service
install -m 0644 "$package_dir/zorvinkel-service.path" /etc/systemd/system/zorvinkel-service.path
install -m 0644 "$package_dir/zorvinkel-service.service" /etc/systemd/system/zorvinkel-service.service
install -m 0755 "$package_dir/zorvinkel-ap" /usr/local/libexec/zorvinkel-ap
install -m 0644 "$package_dir/zorvinkel-ap.path" /etc/systemd/system/zorvinkel-ap.path
install -m 0644 "$package_dir/zorvinkel-ap.service" /etc/systemd/system/zorvinkel-ap.service
# The directory enables the service list; the example file is inactive (wrong extension).
root_dir=$(CDPATH= cd -- "$package_dir/../.." && pwd)
install -d -m 0755 /usr/local/share/doc/zorvinkel
install -m 0644 "$root_dir/LICENSE" "$root_dir/THIRD-PARTY-NOTICES.md" "$root_dir/THIRD-PARTY-LICENSES.md" /usr/local/share/doc/zorvinkel/
install -m 0644 "$root_dir/vendor/bluer/LICENSE" /usr/local/share/doc/zorvinkel/LICENSE-bluer
install -d -m 0755 /etc/zorvinkel/services.d
install -m 0644 "$package_dir/services.d/raspotify.toml.example" /etc/zorvinkel/services.d/raspotify.toml.example
install -d -m 0755 /etc/systemd/system/bluetooth.service.d
install -m 0644 "$package_dir/bluetooth-zorvinkel.conf" /etc/systemd/system/bluetooth.service.d/zorvinkel-recovery.conf
systemctl reload dbus.service
systemctl daemon-reload
systemctl reset-failed bluetooth.service zorvinkeld.service
systemctl enable --now zorvinkel-power@poweroff.path zorvinkel-power@reboot.path
# Without this unit the service list stays display-only.
systemctl enable --now zorvinkel-service.path
# The own Wi-Fi needs a DHCP server; without dnsmasq the service does not offer the feature.
if command -v dnsmasq >/dev/null 2>&1; then
    systemctl enable --now zorvinkel-ap.path
else
    systemctl disable zorvinkel-ap.path 2>/dev/null || true
    echo 'Hinweis: dnsmasq fehlt (apt install dnsmasq-base); "Eigenes WLAN" bleibt deaktiviert.' >&2
fi
systemctl enable --now zorvinkeld.service
