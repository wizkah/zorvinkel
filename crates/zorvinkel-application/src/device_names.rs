//! Display labels never participate in authentication or device selection.
use zorvinkel_core::DiscoveredDevice;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisteredName {
    pub handle: String,
    pub identity: [u8; 32],
    pub hostname: String,
}

pub fn hostname(advertised: &str) -> &str {
    match advertised.rsplit_once('-') {
        Some((name, suffix))
            if !name.is_empty()
                && suffix.len() == 4
                && suffix.bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            name
        }
        _ => advertised,
    }
}
pub fn apply(devices: &mut [DiscoveredDevice], registered: &[RegisteredName]) {
    for device in devices {
        let name = hostname(&device.display_name).to_owned();
        let known = registered
            .iter()
            .find(|entry| entry.handle == device.handle);
        device.display_name = match known {
            Some(known)
                if registered
                    .iter()
                    .any(|other| other.hostname == name && other.identity != known.identity) =>
            {
                let hex = |identity: &[u8; 32]| {
                    identity
                        .iter()
                        .map(|byte| format!("{byte:02X}"))
                        .collect::<String>()
                };
                let key = hex(&known.identity);
                let mut length = 8;
                while length < key.len()
                    && registered.iter().any(|other| {
                        other.hostname == name
                            && other.identity != known.identity
                            && hex(&other.identity)[..length] == key[..length]
                    })
                {
                    length += 2;
                }
                format!("{name} · {}", &key[..length])
            }
            _ => name,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_distinct_registered_identities_with_the_same_hostname_need_a_suffix() {
        let device = |handle: &str| DiscoveredDevice {
            handle: handle.into(),
            display_name: "Pi-967C".into(),
        };
        let first = RegisteredName {
            handle: "a".into(),
            identity: [1; 32],
            hostname: "Pi".into(),
        };
        let mut devices = vec![device("a"), device("b")];
        apply(&mut devices, std::slice::from_ref(&first));
        assert!(devices.iter().all(|device| device.display_name == "Pi"));
        let mut second = first.clone();
        second.handle = "b".into();
        let mut devices = vec![device("a"), device("b")];
        apply(&mut devices, &[first.clone(), second.clone()]);
        assert!(devices.iter().all(|device| device.display_name == "Pi"));
        second.identity = [2; 32];
        let mut devices = vec![device("a")];
        apply(&mut devices, &[first, second]);
        assert_eq!(devices[0].display_name, "Pi · 01010101");
    }
    #[test]
    fn hostname_hyphens_and_colliding_short_keys_are_preserved() {
        assert_eq!(hostname("my-pi-967C"), "my-pi");
        assert_eq!(hostname("my-pi"), "my-pi");
        let first = RegisteredName {
            handle: "a".into(),
            identity: [1; 32],
            hostname: "Pi".into(),
        };
        let mut second = first.clone();
        second.handle = "b".into();
        second.identity[4] = 2;
        let mut devices = vec![DiscoveredDevice {
            handle: "a".into(),
            display_name: "Pi-967C".into(),
        }];
        apply(&mut devices, &[first, second]);
        assert_eq!(devices[0].display_name, "Pi · 0101010101");
    }
}
