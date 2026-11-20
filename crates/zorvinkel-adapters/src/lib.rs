//! Infrastructure implementations; simulations perform no system changes.

pub mod simulation;

#[cfg(any(target_os = "linux", all(test, unix)))]
pub mod linux;

pub mod credentials;
pub mod memory;
#[cfg(any(unix, windows))]
pub mod persistence;
pub mod services;
pub mod system_info;
#[cfg(feature = "security")]
pub mod wifi_cache;
#[cfg(all(windows, feature = "security"))]
mod windows_credentials;
#[cfg(windows)]
mod windows_wifi;

pub mod ble;

#[cfg(all(
    feature = "security",
    any(
        feature = "ble-client",
        all(feature = "ble-server", target_os = "linux")
    )
))]
pub mod ip;

#[cfg(all(
    feature = "security",
    any(
        feature = "ble-client",
        all(feature = "ble-server", target_os = "linux")
    )
))]
pub mod discovery;

pub mod identity;
#[cfg(feature = "security")]
pub mod security;

#[cfg(feature = "tunnel")]
pub mod tunnel_gate;
#[cfg(feature = "tunnel")]
pub mod tunnel_io;
