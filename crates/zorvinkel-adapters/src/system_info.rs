//! Conversion between the application's system information and its wire form.
use zorvinkel_application::system as app;
use zorvinkel_protocol::system as wire;

pub fn to_wire(info: app::SystemInfo) -> wire::SystemInfo {
    wire::SystemInfo {
        version: info.version,
        uptime_seconds: info.uptime_seconds,
        load_milli: info.load_milli,
        memory_total: info.memory_total,
        memory_available: info.memory_available,
        storage_total: info.storage_total,
        storage_free: info.storage_free,
        temperature_milli: info.temperature_milli,
        cpu_mhz: info.cpu_mhz,
        cpu_max_mhz: info.cpu_max_mhz,
        throttle_flags: info.throttle_flags,
        os_name: info.os_name,
        kernel: info.kernel,
        model: info.model,
        hostname: info.hostname,
        addresses: info
            .addresses
            .into_iter()
            .map(|entry| wire::NetworkAddress {
                interface: entry.interface,
                address: entry.address,
            })
            .collect(),
    }
}
pub fn from_wire(info: wire::SystemInfo) -> app::SystemInfo {
    app::SystemInfo {
        version: info.version,
        uptime_seconds: info.uptime_seconds,
        load_milli: info.load_milli,
        memory_total: info.memory_total,
        memory_available: info.memory_available,
        storage_total: info.storage_total,
        storage_free: info.storage_free,
        temperature_milli: info.temperature_milli,
        cpu_mhz: info.cpu_mhz,
        cpu_max_mhz: info.cpu_max_mhz,
        throttle_flags: info.throttle_flags,
        os_name: info.os_name,
        kernel: info.kernel,
        model: info.model,
        hostname: info.hostname,
        addresses: info
            .addresses
            .into_iter()
            .map(|entry| app::NetworkAddress {
                interface: entry.interface,
                address: entry.address,
            })
            .collect(),
    }
}
