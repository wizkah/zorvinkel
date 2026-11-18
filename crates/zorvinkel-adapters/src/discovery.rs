//! mDNS/DNS-SD announcement and search of devices on the local network (ADR-023).
//! Announced values only locate a candidate; the handshake proves the device identity.
use crate::ble::BleResult;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    net::IpAddr,
    time::{Duration, Instant},
};
use zorvinkel_protocol::ip::{PORT, SERVICE_TYPE};

const KEY_FINGERPRINT: &str = "fp";
const KEY_VERSION: &str = "v";
const VERSION: &str = "1";
/// Bytes of the SHA-256 digest of the device key that the announcement carries.
const FINGERPRINT_BYTES: usize = 8;
const MAX_DEVICES: usize = 64;
/// Handles of contacts that were only seen on the network start with this prefix.
const IP_HANDLE_PREFIX: &str = "ip:";

/// A short, non-secret hint that lets a client match an announcement to a stored device key.
pub fn fingerprint(public_key: &[u8]) -> String {
    Sha256::digest(public_key)[..FINGERPRINT_BYTES]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Keeps the announcement alive; dropping it withdraws the service.
pub struct Announcement {
    daemon: ServiceDaemon,
    fullname: String,
}
impl Drop for Announcement {
    fn drop(&mut self) {
        // Goodbye packets are best effort; the daemon stops either way.
        let _ = self.daemon.unregister(&self.fullname);
        let _ = self.daemon.shutdown();
    }
}

/// Announces the IP endpoint. `instance` equals the Bluetooth name, so both transports show the
/// same device. The host name is derived from it to avoid clashing with the system's own mDNS host.
pub fn announce(instance: &str, public_key: &[u8]) -> BleResult<Announcement> {
    let daemon = ServiceDaemon::new()?;
    let host = format!("zorvinkel-{}.local.", host_label(instance));
    let properties = [
        (KEY_VERSION, VERSION.to_owned()),
        (KEY_FINGERPRINT, fingerprint(public_key)),
    ];
    let info = ServiceInfo::new(SERVICE_TYPE, instance, &host, (), PORT, &properties[..])?
        .enable_addr_auto();
    let fullname = info.get_fullname().to_owned();
    daemon.register(info)?;
    Ok(Announcement { daemon, fullname })
}

fn host_label(instance: &str) -> String {
    instance
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .to_ascii_lowercase()
}

/// A device announced on the local network.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpDevice {
    /// Instance name; it matches the Bluetooth name of the same device.
    pub instance: String,
    pub fingerprint: Option<String>,
    pub addresses: Vec<IpAddr>,
    pub port: u16,
}

/// Link-local IPv6 addresses need an interface scope that the announcement does not carry.
fn usable(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => !address.is_loopback() && !address.is_unspecified(),
        IpAddr::V6(address) => {
            !address.is_loopback()
                && !address.is_unspecified()
                && !address.is_multicast()
                && (address.segments()[0] & 0xffc0) != 0xfe80
        }
    }
}

/// The instance label of `fullname` (`<instance>.<service type>`).
fn instance_of(fullname: &str) -> Option<&str> {
    let instance = fullname.strip_suffix(SERVICE_TYPE)?.strip_suffix('.')?;
    (!instance.is_empty() && instance.len() <= 63 && !instance.chars().any(char::is_control))
        .then_some(instance)
}

fn valid_fingerprint(value: &str) -> bool {
    value.len() == FINGERPRINT_BYTES * 2 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Chooses the handle under which each announced device is listed. A device whose fingerprint
/// matches a registered device keeps that device's handle, so Bluetooth and IP share one trust
/// binding; any other device is a new contact under `ip:<instance>`, unless Bluetooth already
/// lists a device of the same name: the first registration then runs over Bluetooth (in radio
/// range), and later searches match the fingerprint. The name only hides a duplicate entry; it
/// grants no trust.
pub fn assign_handles(
    found: Vec<IpDevice>,
    registered: &[zorvinkel_application::device_names::RegisteredName],
    bluetooth_names: &[&str],
) -> Vec<(String, IpDevice)> {
    found
        .into_iter()
        .filter_map(|device| {
            // Several contacts with one identity are one device (e.g. registered once over
            // Bluetooth and once over the network); the Bluetooth contact is preferred. Different
            // identities behind one fingerprint are ambiguous and match nothing.
            let known = device.fingerprint.as_ref().and_then(|hint| {
                let matches: Vec<_> = registered
                    .iter()
                    .filter(|entry| &fingerprint(&entry.identity) == hint)
                    .collect();
                let first = *matches.first()?;
                matches
                    .iter()
                    .all(|entry| entry.identity == first.identity)
                    .then(|| {
                        matches
                            .iter()
                            .find(|entry| !entry.handle.starts_with(IP_HANDLE_PREFIX))
                            .map_or(first, |entry| *entry)
                    })
            });
            match known {
                Some(entry) => Some((entry.handle.clone(), device)),
                None if bluetooth_names.contains(&device.instance.as_str()) => None,
                None => Some((format!("{IP_HANDLE_PREFIX}{}", device.instance), device)),
            }
        })
        .collect()
}

/// Browses for `duration` and returns every device that resolved to at least one usable address.
pub fn browse(duration: Duration) -> BleResult<Vec<IpDevice>> {
    let daemon = ServiceDaemon::new()?;
    let events = daemon.browse(SERVICE_TYPE)?;
    let deadline = Instant::now() + duration;
    let mut found = BTreeMap::<String, IpDevice>::new();
    while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        match events.recv_timeout(remaining) {
            Ok(ServiceEvent::ServiceResolved(service)) => {
                let Some(instance) = instance_of(service.get_fullname()) else {
                    continue;
                };
                let addresses: Vec<IpAddr> = service
                    .get_addresses()
                    .iter()
                    .map(|address| address.to_ip_addr())
                    .filter(usable)
                    .collect();
                if addresses.is_empty() || service.get_port() == 0 {
                    continue;
                }
                let fingerprint = service
                    .get_property_val_str(KEY_FINGERPRINT)
                    .filter(|value| valid_fingerprint(value))
                    .map(str::to_ascii_lowercase);
                if found.len() >= MAX_DEVICES && !found.contains_key(instance) {
                    continue;
                }
                found.insert(
                    instance.to_owned(),
                    IpDevice {
                        instance: instance.to_owned(),
                        fingerprint,
                        addresses,
                        port: service.get_port(),
                    },
                );
            }
            Ok(_) => (),
            Err(_) => break,
        }
    }
    let _ = daemon.shutdown();
    Ok(found.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fingerprint_is_a_short_hex_digest() {
        let value = fingerprint(&[7; 32]);
        assert!(valid_fingerprint(&value));
        assert_ne!(value, fingerprint(&[8; 32]));
    }
    #[test]
    fn instance_names_are_read_from_full_names() {
        assert_eq!(
            instance_of("Pi-967C._zorvinkel._tcp.local."),
            Some("Pi-967C")
        );
        assert_eq!(instance_of("Pi._other._tcp.local."), None);
        assert_eq!(instance_of("._zorvinkel._tcp.local."), None);
    }
    #[test]
    fn only_routable_addresses_are_candidates() {
        assert!(usable(&"192.168.1.5".parse().unwrap()));
        assert!(usable(&"fd00::5".parse().unwrap()));
        assert!(!usable(&"fe80::1".parse().unwrap()));
        assert!(!usable(&"127.0.0.1".parse().unwrap()));
        assert!(!usable(&"::1".parse().unwrap()));
    }
    #[test]
    fn registered_fingerprints_reuse_the_registered_handle() {
        use zorvinkel_application::device_names::RegisteredName;
        let device = |instance: &str, key: [u8; 32]| IpDevice {
            instance: instance.into(),
            fingerprint: Some(fingerprint(&key)),
            addresses: vec!["192.168.1.5".parse().unwrap()],
            port: PORT,
        };
        let registered = [RegisteredName {
            handle: "ble-handle".into(),
            identity: [1; 32],
            hostname: "Pi".into(),
        }];
        let handles = assign_handles(
            vec![device("Pi-967C", [1; 32]), device("Pi-AAAA", [2; 32])],
            &registered,
            &[],
        );
        assert_eq!(handles[0].0, "ble-handle");
        assert_eq!(handles[1].0, "ip:Pi-AAAA");
    }
    #[test]
    fn contacts_sharing_one_identity_merge_into_the_bluetooth_contact() {
        use zorvinkel_application::device_names::RegisteredName;
        let contact = |handle: &str, key: [u8; 32]| RegisteredName {
            handle: handle.into(),
            identity: key,
            hostname: "Pi".into(),
        };
        let device = |key: [u8; 32]| IpDevice {
            instance: "Pi-967C".into(),
            fingerprint: Some(fingerprint(&key)),
            addresses: vec!["192.168.1.5".parse().unwrap()],
            port: PORT,
        };
        let registered = [contact("ip:Pi-967C", [1; 32]), contact("AA:BB", [1; 32])];
        let handles = assign_handles(vec![device([1; 32])], &registered, &[]);
        assert_eq!(handles[0].0, "AA:BB");
        // Without a Bluetooth contact the first registered contact is used.
        let only_ip = [contact("ip:Pi-967C", [1; 32])];
        let handles = assign_handles(vec![device([1; 32])], &only_ip, &[]);
        assert_eq!(handles[0].0, "ip:Pi-967C");
    }
    #[test]
    fn an_unregistered_device_seen_over_bluetooth_is_listed_once() {
        use zorvinkel_application::device_names::RegisteredName;
        let device = |instance: &str, key: [u8; 32]| IpDevice {
            instance: instance.into(),
            fingerprint: Some(fingerprint(&key)),
            addresses: vec!["192.168.1.5".parse().unwrap()],
            port: PORT,
        };
        let registered = [RegisteredName {
            handle: "ble-handle".into(),
            identity: [1; 32],
            hostname: "Pi".into(),
        }];
        let handles = assign_handles(
            vec![
                device("Pi-NEW1", [2; 32]),
                device("Pi-IPON", [3; 32]),
                device("Pi-967C", [1; 32]),
            ],
            &registered,
            &["Pi-NEW1", "Pi-967C"],
        );
        // The unregistered twin of a Bluetooth entry is hidden, an IP-only device stays and a
        // registered device keeps its handle even when Bluetooth lists it as well.
        let listed: Vec<_> = handles.iter().map(|(handle, _)| handle.as_str()).collect();
        assert_eq!(listed, ["ip:Pi-IPON", "ble-handle"]);
    }
    /// Needs multicast on a real network interface, so it does not run in CI.
    #[test]
    #[ignore]
    fn an_announced_device_is_found_by_the_search() {
        let key = [9; 32];
        let _announcement = announce("Test-0001", &key).unwrap();
        let found = browse(Duration::from_secs(6)).unwrap();
        let device = found
            .iter()
            .find(|device| device.instance == "Test-0001")
            .expect("announced device found");
        assert_eq!(device.port, PORT);
        assert_eq!(
            device.fingerprint.as_deref(),
            Some(fingerprint(&key).as_str())
        );
    }
    #[test]
    fn host_labels_are_dns_safe() {
        assert_eq!(host_label("My Pi_967C"), "my-pi-967c");
    }
}
