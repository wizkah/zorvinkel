# Active scanning on Windows

Source: crates.io `btleplug` 0.13.5, BSD-3-Clause (dual MIT/Apache parts). Upstream source and license are retained.

`src/winrtble/ble/watcher.rs` sets the advertisement watcher to active scanning. Windows scans passively by default and then never receives scan responses, so peripherals whose advertisement is full (128-bit service UUID plus name, as BlueZ sends it) show no name. Remove this patch when upstream scans actively.
