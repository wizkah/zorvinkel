# BlueZ 5.50 compatibility

Source: crates.io `bluer` 0.17.4, BSD-2-Clause. Upstream source and license are retained.

`src/gatt/local.rs` accepts the legacy `MTU` option alongside `mtu`. Value requests without either option use the minimum ATT MTU of 23. Acquisition still requires an explicit MTU. Invalid types and values below 23 are rejected. Device identity remains mandatory.

Regression tests cover legacy and current options, malformed MTUs and missing device paths. Remove this patch when an upstream release provides equivalent compatibility.

`ApplicationHandle::unregister` and `AdvertisementHandle::unregister` await the existing asynchronous cleanup and report its result. The native service uses them before stopping its runtime, so graceful shutdown does not cancel pending D-Bus unregistration. Dropping a handle retains the upstream behavior.
