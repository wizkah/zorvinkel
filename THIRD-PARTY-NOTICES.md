# Third-Party Notices

Zorvinkel is licensed under GPL-3.0-or-later (see `LICENSE`). It contains or
uses the following third-party components. Each keeps its own license; the
full texts accompany the respective source packages (`~/.cargo/registry` or
`vendor/bluer/LICENSE`) and must accompany binary distributions.

## Vendored bluer (`vendor/bluer`)

Source: crates.io `bluer` 0.17.4, with local patches (`vendor/bluer/PATCHES.md`).
The full license text is in `vendor/bluer/LICENSE`:

- BSD-2-Clause: Sebastian Urban, BlueR contributors, University of Szeged
  (Attila Dusnoki).
- BSD-3-Clause: Nordic Semiconductor ASA, database `bluetooth-numbers-database`.
- GPL-2.0: API documentation taken from BlueZ (doc comments only).

## Vendored btleplug (`vendor/btleplug`)

Source: crates.io `btleplug` 0.13.5, BSD-3-Clause (parts MIT/Apache-2.0), with a local
patch (`vendor/btleplug/PATCHES.md`). License text: `vendor/btleplug/LICENSE.md`.

## Fonts (GUI)

`epaint_default_fonts` bundles fonts under OFL-1.1 and Ubuntu-font-1.0
(licenses: package `epaint_default_fonts`).

## Other dependencies

The remaining crates are licensed under MIT, Apache-2.0 (also with LLVM-exception),
BSD-1/2/3-Clause, BSL-1.0, ISC, 0BSD, Unicode-3.0, Unlicense or Zlib.
`r-efi` is `MIT OR Apache-2.0 OR LGPL-2.1-or-later`; MIT or Apache-2.0 is used.
Copyright notices and license texts are in the source packages.

The full license texts are in `THIRD-PARTY-LICENSES.md` (generated with
`packaging/licenses.sh` from `about.toml` and `about.hbs`; regenerate after
changes to `Cargo.lock`). Check and list:

    cargo deny --all-features check licenses
    cargo deny --all-features list
