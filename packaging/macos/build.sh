#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$root"
output=${1:-target/Zorvinkel.app}
[ ! -e "$output" ] || { echo "$output existiert bereits." >&2; exit 1; }
cargo build --release --locked -p zorvinkel-client --features gui,ble-client,security
packaging/macos/package.sh target/release/zorvinkel-client "$output"
