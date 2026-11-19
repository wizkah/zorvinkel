//! SSH host keys the user trusted, one line per device (ADR-030: trust on first use).
use std::{collections::BTreeMap, path::PathBuf};

/// What the stored key says about the key a device presents now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trust {
    Unknown,
    Known,
    Changed,
}

pub struct HostKeyStore {
    path: Option<PathBuf>,
    fingerprints: BTreeMap<String, String>,
}
impl HostKeyStore {
    const FILE: &'static str = "ssh_host_keys";

    pub fn in_directory(directory: &std::path::Path) -> Self {
        let path = directory.join(Self::FILE);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        Self {
            fingerprints: parse(&text),
            path: Some(path),
        }
    }
    /// For runs that must not touch the client's data, such as the simulation.
    pub fn none() -> Self {
        Self {
            path: None,
            fingerprints: BTreeMap::new(),
        }
    }
    pub fn stored(&self, device: &str) -> Option<&str> {
        self.fingerprints.get(device).map(String::as_str)
    }
    pub fn check(&self, device: &str, fingerprint: &str) -> Trust {
        match self.stored(device) {
            None => Trust::Unknown,
            Some(stored) if stored == fingerprint => Trust::Known,
            Some(_) => Trust::Changed,
        }
    }
    pub fn remember(&mut self, device: &str, fingerprint: &str) {
        if !is_field(device) || !is_field(fingerprint) {
            return;
        }
        self.fingerprints
            .insert(device.to_owned(), fingerprint.to_owned());
        self.save();
    }
    fn save(&self) {
        let Some(path) = &self.path else {
            return;
        };
        if let Err(error) = std::fs::write(path, serialize(&self.fingerprints)) {
            eprintln!("SSH host keys not saved: {error}");
        }
    }
}

/// Fields are separated by a tab and lines by a newline, so neither may occur inside.
fn is_field(text: &str) -> bool {
    !text.is_empty() && !text.contains(['\t', '\n', '\r'])
}
fn parse(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|line| line.split_once('\t'))
        .filter(|(device, fingerprint)| is_field(device) && is_field(fingerprint))
        .map(|(device, fingerprint)| (device.to_owned(), fingerprint.to_owned()))
        .collect()
}
fn serialize(fingerprints: &BTreeMap<String, String>) -> String {
    fingerprints
        .iter()
        .map(|(device, fingerprint)| format!("{device}\t{fingerprint}\n"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_device_is_unknown_until_remembered() {
        let mut store = HostKeyStore::none();
        assert_eq!(store.check("pi", "SHA256:a"), Trust::Unknown);
        store.remember("pi", "SHA256:a");
        assert_eq!(store.check("pi", "SHA256:a"), Trust::Known);
        assert_eq!(store.check("pi", "SHA256:b"), Trust::Changed);
        assert_eq!(store.check("other", "SHA256:a"), Trust::Unknown);
    }
    #[test]
    fn keys_are_kept_in_the_data_directory() {
        let directory =
            std::env::temp_dir().join(format!("zorvinkel-host-keys-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut store = HostKeyStore::in_directory(&directory);
        store.remember("pi", "SHA256:a");
        assert_eq!(
            HostKeyStore::in_directory(&directory).check("pi", "SHA256:a"),
            Trust::Known
        );
        std::fs::remove_dir_all(&directory).unwrap();
    }
    #[test]
    fn unusual_names_are_not_stored_and_damaged_lines_are_skipped() {
        let mut store = HostKeyStore::none();
        store.remember("a\tb", "SHA256:a");
        store.remember("pi", "");
        assert_eq!(store.stored("a\tb"), None);
        assert_eq!(store.stored("pi"), None);
        let parsed = parse("no tab here\npi\tSHA256:a\n\t\n");
        assert_eq!(parsed.len(), 1);
    }
}
