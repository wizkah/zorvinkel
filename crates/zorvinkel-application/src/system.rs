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
impl SystemInfo {
    /// The address the client uses to reach the device's `sshd`: a routable IPv4 address if there
    /// is one, otherwise a global IPv6 address.
    pub fn ssh_address(&self) -> Option<IpAddr> {
        let usable = |address: &&NetworkAddress| match address.address {
            IpAddr::V4(ip) => !ip.is_loopback() && !ip.is_link_local(),
            // A link-local IPv6 address needs a scope that the device cannot tell us.
            IpAddr::V6(ip) => !ip.is_loopback() && (ip.segments()[0] & 0xffc0) != 0xfe80,
        };
        let mut usable_addresses = self.addresses.iter().filter(usable);
        let first_v4 = self
            .addresses
            .iter()
            .filter(usable)
            .find(|address| address.address.is_ipv4());
        first_v4
            .or_else(|| usable_addresses.next())
            .map(|address| address.address)
    }
}
pub trait SystemInfoSource {
    fn read(&mut self) -> Result<SystemInfo, crate::PortError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(addresses: &[&str]) -> SystemInfo {
        SystemInfo {
            version: String::new(),
            uptime_seconds: 0,
            load_milli: [0; 3],
            memory_total: 0,
            memory_available: 0,
            storage_total: 0,
            storage_free: 0,
            temperature_milli: None,
            cpu_mhz: None,
            cpu_max_mhz: None,
            throttle_flags: None,
            os_name: None,
            kernel: None,
            model: None,
            hostname: None,
            addresses: addresses
                .iter()
                .map(|text| NetworkAddress {
                    interface: "eth0".into(),
                    address: text.parse().unwrap(),
                })
                .collect(),
        }
    }

    #[test]
    fn ssh_address_prefers_a_routable_ipv4_address() {
        let info = info(&[
            "fe80::1",
            "2001:db8::7",
            "127.0.0.1",
            "169.254.3.4",
            "192.168.1.20",
        ]);
        assert_eq!(info.ssh_address(), "192.168.1.20".parse().ok());
    }
    #[test]
    fn ssh_address_falls_back_to_a_global_ipv6_address() {
        assert_eq!(
            info(&["fe80::1", "2001:db8::7"]).ssh_address(),
            "2001:db8::7".parse().ok()
        );
    }
    #[test]
    fn local_addresses_alone_are_no_ssh_address() {
        assert_eq!(info(&["127.0.0.1", "::1", "fe80::2"]).ssh_address(), None);
        assert_eq!(info(&[]).ssh_address(), None);
    }
}
