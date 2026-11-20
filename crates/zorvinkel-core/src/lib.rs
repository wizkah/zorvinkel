//! Hardware-independent device and network state.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoveredDevice {
    /// A discovery handle, not proof of device identity.
    pub handle: String,
    pub display_name: String,
    /// The latest search found the device over Bluetooth.
    pub via_ble: bool,
    /// The latest search found the device announced on the local network.
    pub via_ip: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkStatus {
    Disconnected,
    WifiConnected { ssid: String },
    AccessPointActive { ssid: String },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IpReachability {
    #[default]
    Unverified,
    Authenticated,
    Unreachable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceStatus {
    pub network: NetworkStatus,
    pub ip_reachability: IpReachability,
}

pub mod network;
pub mod registry;

pub mod trust;
