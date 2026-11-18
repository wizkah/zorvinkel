//! Read-only system information from `/proc`, `/sys` and `df`.
use std::{fs, net::IpAddr, path::PathBuf, process::Command};
use zorvinkel_application::{
    PortError,
    system::{NetworkAddress, SystemInfo, SystemInfoSource},
};

const SERVICE_VERSION: &str = env!("CARGO_PKG_VERSION");
const STORAGE_PATH: &str = "/";
const TEMPERATURE_FILE: &str = "/sys/class/thermal/thermal_zone0/temp";

pub struct SystemReader {
    proc_directory: PathBuf,
}
impl Default for SystemReader {
    fn default() -> Self {
        Self {
            proc_directory: "/proc".into(),
        }
    }
}
impl SystemInfoSource for SystemReader {
    fn read(&mut self) -> Result<SystemInfo, PortError> {
        let proc_file = |name: &str| {
            fs::read_to_string(self.proc_directory.join(name)).map_err(|_| PortError::Unavailable)
        };
        let (memory_total, memory_available) = parse_memory(&proc_file("meminfo")?)?;
        let (storage_total, storage_free) = parse_storage(&run_df()?)?;
        // Everything below is informational: a missing source must not hide the other values.
        let text = |path: &str| fs::read_to_string(path).ok();
        let frequency = |name: &str| {
            text(&format!("/sys/devices/system/cpu/cpu0/cpufreq/{name}"))
                .and_then(|text| parse_frequency_mhz(&text))
        };
        Ok(SystemInfo {
            version: SERVICE_VERSION.into(),
            uptime_seconds: parse_uptime(&proc_file("uptime")?)?,
            load_milli: parse_load(&proc_file("loadavg")?)?,
            memory_total,
            memory_available,
            storage_total,
            storage_free,
            temperature_milli: text(TEMPERATURE_FILE).and_then(|text| parse_temperature(&text)),
            cpu_mhz: frequency("scaling_cur_freq"),
            cpu_max_mhz: frequency("cpuinfo_max_freq"),
            throttle_flags: command_output("vcgencmd", &["get_throttled"])
                .and_then(|output| parse_throttled(&output)),
            os_name: text("/etc/os-release").and_then(|text| parse_os_name(&text)),
            kernel: text("/proc/sys/kernel/osrelease").and_then(|text| single_line(&text)),
            model: text("/sys/firmware/devicetree/base/model").and_then(|text| single_line(&text)),
            hostname: text("/proc/sys/kernel/hostname").and_then(|text| single_line(&text)),
            addresses: command_output("ip", &["-o", "addr", "show", "scope", "global"])
                .map(|output| parse_addresses(&output))
                .unwrap_or_default(),
        })
    }
}
fn command_output(program: &str, arguments: &[&str]) -> Option<String> {
    let output = Command::new(program).args(arguments).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())?
}
fn run_df() -> Result<String, PortError> {
    command_output("df", &["-Pk", STORAGE_PATH]).ok_or(PortError::Unavailable)
}

/// `/proc/uptime`: "<uptime seconds> <idle seconds>".
pub fn parse_uptime(text: &str) -> Result<u64, PortError> {
    let seconds: f64 = text
        .split_whitespace()
        .next()
        .and_then(|value| value.parse().ok())
        .ok_or(PortError::Unavailable)?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(PortError::Unavailable);
    }
    Ok(seconds as u64)
}
/// `/proc/loadavg`: "<1 min> <5 min> <15 min> ...".
pub fn parse_load(text: &str) -> Result<[u32; 3], PortError> {
    let mut values = text.split_whitespace().take(3).map(|value| {
        value
            .parse::<f64>()
            .ok()
            .filter(|load| load.is_finite() && *load >= 0.0)
            .map(|load| (load * 1000.0).round() as u32)
    });
    let mut next = || values.next().flatten().ok_or(PortError::Unavailable);
    Ok([next()?, next()?, next()?])
}
/// `/proc/meminfo`: values in kiB. Returns total and available bytes.
pub fn parse_memory(text: &str) -> Result<(u64, u64), PortError> {
    let kib = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
            .and_then(|rest| rest.split_whitespace().next()?.parse::<u64>().ok())
            .and_then(|value| value.checked_mul(1024))
    };
    let total = kib("MemTotal").ok_or(PortError::Unavailable)?;
    // Kernels before 3.14 lack `MemAvailable`; then free memory is the best known value.
    let available = kib("MemAvailable")
        .or_else(|| kib("MemFree"))
        .ok_or(PortError::Unavailable)?;
    Ok((total, available.min(total)))
}
/// `df -Pk` output: a header and one line "<fs> <1K-blocks> <used> <available> ...".
/// Returns total and available bytes; the available figure is what unprivileged users can use.
pub fn parse_storage(text: &str) -> Result<(u64, u64), PortError> {
    let columns: Vec<&str> = text
        .lines()
        .nth(1)
        .ok_or(PortError::Unavailable)?
        .split_whitespace()
        .collect();
    let kib = |index: usize| columns.get(index)?.parse::<u64>().ok()?.checked_mul(1024);
    let total = kib(1).ok_or(PortError::Unavailable)?;
    let free = kib(3).ok_or(PortError::Unavailable)?;
    Ok((total, free.min(total)))
}
/// Sysfs reports thousandths of a degree Celsius.
pub fn parse_temperature(text: &str) -> Option<i32> {
    text.trim().parse().ok()
}

/// Sysfs reports kHz.
pub fn parse_frequency_mhz(text: &str) -> Option<u32> {
    text.trim().parse::<u32>().ok().map(|khz| khz / 1000)
}
/// `vcgencmd get_throttled`: "throttled=0x50005".
pub fn parse_throttled(text: &str) -> Option<u32> {
    let value = text.trim().strip_prefix("throttled=0x")?;
    u32::from_str_radix(value, 16).ok()
}
/// The `PRETTY_NAME` of `/etc/os-release`, without quotes.
pub fn parse_os_name(text: &str) -> Option<String> {
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("PRETTY_NAME="))?;
    single_line(value.trim_matches(['"', '\'']))
}
/// Kernel files end with a newline and the device tree model with a NUL byte.
fn single_line(text: &str) -> Option<String> {
    let text = text.trim_matches(|c: char| c.is_whitespace() || c == '\0');
    (!text.is_empty() && text.len() <= 64 && !text.chars().any(char::is_control))
        .then(|| text.to_owned())
}
/// `ip -o addr show scope global`: "<n>: <interface> <family> <address>/<prefix> ...".
pub fn parse_addresses(text: &str) -> Vec<NetworkAddress> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace().skip(1);
            let interface = fields.next()?.split('@').next()?;
            fields.next().filter(|family| family.starts_with("inet"))?;
            let address: IpAddr = fields.next()?.split('/').next()?.parse().ok()?;
            (interface.len() <= 15).then(|| NetworkAddress {
                interface: interface.to_owned(),
                address,
            })
        })
        .take(8)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uptime_load_and_temperature_are_parsed() {
        assert_eq!(parse_uptime("12345.67 9876.54\n"), Ok(12345));
        assert!(parse_uptime("").is_err());
        assert!(parse_uptime("-1 0").is_err());
        assert_eq!(parse_load("0.05 1.25 0.00 1/80 1234\n"), Ok([50, 1250, 0]));
        assert!(parse_load("0.05 x 0.00").is_err());
        assert!(parse_load("0.05 0.1").is_err());
        assert_eq!(parse_temperature("48312\n"), Some(48312));
        assert_eq!(parse_temperature("hot"), None);
    }
    #[test]
    fn memory_uses_available_and_falls_back_to_free() {
        let modern = "MemTotal:  425000 kB\nMemFree: 1000 kB\nMemAvailable: 300000 kB\n";
        assert_eq!(parse_memory(modern), Ok((425_000 * 1024, 300_000 * 1024)));
        let old = "MemTotal: 400 kB\nMemFree: 100 kB\n";
        assert_eq!(parse_memory(old), Ok((400 * 1024, 100 * 1024)));
        assert!(parse_memory("MemFree: 100 kB\n").is_err());
    }
    #[test]
    fn storage_comes_from_the_portable_df_layout() {
        let output = "Filesystem 1024-blocks Used Available Capacity Mounted on\n/dev/root 15000000 4000000 10500000 28% /\n";
        assert_eq!(
            parse_storage(output),
            Ok((15_000_000 * 1024, 10_500_000 * 1024))
        );
        assert!(parse_storage("Filesystem 1024-blocks\n").is_err());
        assert!(parse_storage("").is_err());
    }
    #[test]
    fn identity_and_network_values_are_parsed() {
        assert_eq!(parse_frequency_mhz("700000\n"), Some(700));
        assert_eq!(parse_throttled("throttled=0x50005\n"), Some(0x50005));
        assert_eq!(parse_throttled("error=1"), None);
        let release =
            "NAME=\"Raspbian GNU/Linux\"\nPRETTY_NAME=\"Raspbian GNU/Linux 10 (buster)\"\n";
        assert_eq!(
            parse_os_name(release).as_deref(),
            Some("Raspbian GNU/Linux 10 (buster)")
        );
        assert_eq!(parse_os_name("NAME=x\n"), None);
        assert_eq!(
            single_line("Raspberry Pi Zero W\0").as_deref(),
            Some("Raspberry Pi Zero W")
        );
        assert_eq!(single_line("\n"), None);
        let output = "3: wlan0    inet 192.168.1.42/24 brd 192.168.1.255 scope global dynamic wlan0\n\
            3: wlan0    inet6 2001:db8::5/64 scope global dynamic\n\
            4: eth0@if9    inet 10.0.0.2/8 scope global eth0\n\
            5: wlan1    link/ether aa:bb scope global\n";
        let addresses = parse_addresses(output);
        assert_eq!(addresses.len(), 3);
        assert_eq!(addresses[0].interface, "wlan0");
        assert_eq!(
            addresses[0].address,
            "192.168.1.42".parse::<IpAddr>().unwrap()
        );
        assert_eq!(
            addresses[1].address,
            "2001:db8::5".parse::<IpAddr>().unwrap()
        );
        assert_eq!(addresses[2].interface, "eth0");
    }
    #[test]
    fn the_running_system_can_be_read() {
        if !std::path::Path::new("/proc/meminfo").exists() {
            return;
        }
        let info = SystemReader::default().read().unwrap();
        assert!(info.memory_total > 0 && info.storage_total > 0);
        assert!(!info.version.is_empty());
    }
}
