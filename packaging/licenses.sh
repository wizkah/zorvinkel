#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
cargo about generate --all-features --locked about.hbs | tr -d '\r' > THIRD-PARTY-LICENSES.md
