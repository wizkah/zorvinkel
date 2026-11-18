//! Read-only system information of the device. Authorization is the caller's duty.

use std::net::IpAddr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkAddress {
    pub interface: String,
    pub address: IpAddr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemInfo {
    /// Version of the device service.
    pub version: String,
    pub uptime_seconds: u64,
    /// Load averages over 1, 5 and 15 minutes in thousandths.
    pub load_milli: [u32; 3],
    pub memory_total: u64,
    pub memory_available: u64,
    pub storage_total: u64,
    pub storage_free: u64,
    /// SoC temperature in thousandths of a degree Celsius, when the device has a sensor.
    pub temperature_milli: Option<i32>,
    pub cpu_mhz: Option<u32>,
    pub cpu_max_mhz: Option<u32>,
    /// Raspberry Pi firmware flags (`vcgencmd get_throttled`), when the device has them.
    pub throttle_flags: Option<u32>,
    pub os_name: Option<String>,
    pub kernel: Option<String>,
    pub model: Option<String>,
    pub hostname: Option<String>,
    pub addresses: Vec<NetworkAddress>,
}
pub trait SystemInfoSource {
    fn read(&mut self) -> Result<SystemInfo, crate::PortError>;
}
