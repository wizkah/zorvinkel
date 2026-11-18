#!/bin/sh
set -eu
[ "$#" = 2 ] || { echo 'Usage: package.sh client-binary output.app' >&2; exit 1; }
[ -x "$1" ]
[ ! -e "$2" ] || { echo 'Output already exists.' >&2; exit 1; }
icon_work=$(mktemp -d "${TMPDIR:-/tmp}/zorvinkel-icon.XXXXXX")
trap 'rm -rf "$icon_work"' EXIT HUP INT TERM
mkdir -p "$2/Contents/MacOS" "$2/Contents/Resources" "$icon_work/AppIcon.iconset"
"$1" --export-app-icon "$icon_work/master.png"
for size in 16 32 128 256 512; do
    for scale in 1 2; do
        suffix=
        [ "$scale" = 1 ] || suffix=@2x
        pixels=$((size * scale))
        sips -z "$pixels" "$pixels" "$icon_work/master.png" --out "$icon_work/AppIcon.iconset/icon_${size}x${size}${suffix}.png" >/dev/null
    done
done
iconutil -c icns "$icon_work/AppIcon.iconset" -o "$2/Contents/Resources/AppIcon.icns"
cp "$1" "$2/Contents/MacOS/zorvinkel-client"
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cp "$root/LICENSE" "$root/THIRD-PARTY-NOTICES.md" "$root/THIRD-PARTY-LICENSES.md" "$2/Contents/Resources/"
cp -R "$root/vendor/bluer/LICENSE" "$2/Contents/Resources/LICENSE-bluer"
cat > "$2/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>zorvinkel-client</string>
<key>CFBundleIdentifier</key><string>org.zorvinkel.client</string>
<key>CFBundleName</key><string>Zorvinkel</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleIconFile</key><string>AppIcon.icns</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>NSBluetoothAlwaysUsageDescription</key><string>Zorvinkel findet und verbindet dein Gerät in der Nähe.</string>
</dict></plist>
PLIST
codesign --force --sign - "$2"
