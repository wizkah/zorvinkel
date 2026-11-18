//! Client-side cache of Wi-Fi passwords that were accepted by a device. All entries live in
//! one protected OS item, so the system asks for approval at most once for the whole area.
use zeroize::Zeroizing;
use zorvinkel_application::setup::{SetupError, WifiPasswordCache, validate_wifi};

const HEADER: &[u8] = b"ZORVWC\x01";
const MAX_ENTRIES: usize = 64;

/// One opaque secret item; missing is distinct from an access failure.
pub(crate) trait BlobVault {
    fn read(&mut self) -> Result<Option<Zeroizing<Vec<u8>>>, SetupError>;
    fn write(&mut self, value: &[u8]) -> Result<(), SetupError>;
}

pub(crate) struct VaultWifiCache<V>(V);

/// The platform vault; `None` where no secure storage is implemented yet.
#[cfg(target_os = "macos")]
pub fn system_wifi_cache() -> Option<impl WifiPasswordCache> {
    Some(VaultWifiCache(macos::KeychainBlob))
}
#[cfg(windows)]
pub fn system_wifi_cache() -> Option<impl WifiPasswordCache> {
    Some(VaultWifiCache(windows::CredentialBlob))
}
#[cfg(not(any(target_os = "macos", windows)))]
pub fn system_wifi_cache() -> Option<impl WifiPasswordCache> {
    None::<VaultWifiCache<Unsupported>>
}
#[cfg(not(any(target_os = "macos", windows)))]
pub(crate) struct Unsupported;
#[cfg(not(any(target_os = "macos", windows)))]
impl BlobVault for Unsupported {
    fn read(&mut self) -> Result<Option<Zeroizing<Vec<u8>>>, SetupError> {
        Err(SetupError::CredentialsUnavailable)
    }
    fn write(&mut self, _: &[u8]) -> Result<(), SetupError> {
        Err(SetupError::CredentialsUnavailable)
    }
}

type Entries = Vec<(String, Zeroizing<String>)>;

fn decode(bytes: &[u8]) -> Option<Entries> {
    let mut rest = bytes.strip_prefix(HEADER)?;
    let mut entries = Vec::new();
    while !rest.is_empty() {
        let mut field = || {
            let (&length, tail) = rest.split_first()?;
            let (value, tail) = tail.split_at_checked(length.into())?;
            rest = tail;
            std::str::from_utf8(value).ok()
        };
        let ssid = field()?.to_owned();
        let password = Zeroizing::new(field()?.to_owned());
        validate_wifi(&ssid, &password).ok()?;
        entries.push((ssid, password));
        if entries.len() > MAX_ENTRIES {
            return None;
        }
    }
    Some(entries)
}
fn encode(entries: &Entries) -> Zeroizing<Vec<u8>> {
    let mut bytes = Zeroizing::new(HEADER.to_vec());
    for (ssid, password) in entries {
        for value in [ssid.as_bytes(), password.as_bytes()] {
            // Lengths were validated to fit 32 and 63 bytes.
            bytes.push(value.len() as u8);
            bytes.extend_from_slice(value);
        }
    }
    bytes
}

impl<V: BlobVault> VaultWifiCache<V> {
    fn entries(&mut self) -> Result<Entries, SetupError> {
        match self.0.read()? {
            None => Ok(Vec::new()),
            Some(bytes) => decode(&bytes).ok_or(SetupError::CredentialsUnavailable),
        }
    }
}
impl<V: BlobVault> WifiPasswordCache for VaultWifiCache<V> {
    fn cached_password(&mut self, ssid: &str) -> Result<Option<String>, SetupError> {
        Ok(self
            .entries()?
            .into_iter()
            .find(|(name, _)| name == ssid)
            .map(|(_, password)| (*password).clone()))
    }
    fn remember(&mut self, ssid: &str, password: &str) -> Result<(), SetupError> {
        validate_wifi(ssid, password)?;
        let mut entries = self.entries()?;
        entries.retain(|(name, _)| name != ssid);
        if entries.len() >= MAX_ENTRIES {
            entries.remove(0);
        }
        entries.push((ssid.to_owned(), Zeroizing::new(password.to_owned())));
        self.0.write(&encode(&entries))
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use security_framework::os::macos::keychain::SecKeychain;
    use security_framework_sys::base::errSecItemNotFound;
    const SERVICE: &str = "org.zorvinkel.client.wifi";
    const ACCOUNT: &str = "networks";

    pub(super) struct KeychainBlob;
    impl BlobVault for KeychainBlob {
        fn read(&mut self) -> Result<Option<Zeroizing<Vec<u8>>>, SetupError> {
            let keychain =
                SecKeychain::default().map_err(|_| SetupError::CredentialsUnavailable)?;
            match keychain.find_generic_password(SERVICE, ACCOUNT) {
                Ok((password, _)) => Ok(Some(Zeroizing::new(password.to_vec()))),
                Err(error) if error.code() == errSecItemNotFound => Ok(None),
                Err(_) => Err(SetupError::CredentialsUnavailable),
            }
        }
        fn write(&mut self, value: &[u8]) -> Result<(), SetupError> {
            let keychain =
                SecKeychain::default().map_err(|_| SetupError::CredentialsUnavailable)?;
            match keychain.find_generic_password(SERVICE, ACCOUNT) {
                Ok((_, mut item)) => item.set_password(value),
                Err(error) if error.code() == errSecItemNotFound => {
                    keychain.add_generic_password(SERVICE, ACCOUNT, value)
                }
                Err(error) => Err(error),
            }
            .map_err(|_| SetupError::CredentialsUnavailable)
        }
    }
}

/// A Credential Manager blob holds 2560 bytes, so the cache is split over a few items. The
/// first byte of every item is the chunk count, which makes a torn write detectable; such a
/// state is unavailable rather than guessed.
#[cfg(windows)]
mod windows {
    use super::*;
    use crate::windows_credentials::{self as credentials, MAX_BLOB};
    const TARGET: &str = "org.zorvinkel.client.wifi/networks";
    const CHUNK: usize = MAX_BLOB - 1;
    /// 64 entries of at most 97 bytes fit into three chunks.
    const MAX_CHUNKS: usize = 4;

    fn target(index: usize) -> String {
        format!("{TARGET}/{index}")
    }
    fn split(value: &[u8]) -> Option<Vec<Zeroizing<Vec<u8>>>> {
        let count = value.len().div_ceil(CHUNK).max(1);
        if count > MAX_CHUNKS {
            return None;
        }
        let mut chunks = value.chunks(CHUNK).collect::<Vec<_>>();
        chunks.resize(count, &[]);
        Some(
            chunks
                .into_iter()
                .map(|part| {
                    let mut item = Zeroizing::new(vec![count as u8]);
                    item.extend_from_slice(part);
                    item
                })
                .collect(),
        )
    }
    fn join(chunks: &[Zeroizing<Vec<u8>>]) -> Option<Zeroizing<Vec<u8>>> {
        let mut value = Zeroizing::new(Vec::new());
        for chunk in chunks {
            let (&count, part) = chunk.split_first()?;
            if usize::from(count) != chunks.len() {
                return None;
            }
            value.extend_from_slice(part);
        }
        Some(value)
    }

    pub(super) struct CredentialBlob;
    impl BlobVault for CredentialBlob {
        fn read(&mut self) -> Result<Option<Zeroizing<Vec<u8>>>, SetupError> {
            let unavailable = |_| SetupError::CredentialsUnavailable;
            let Some(first) = credentials::read(&target(0)).map_err(unavailable)? else {
                return Ok(None);
            };
            let count = usize::from(*first.first().ok_or(SetupError::CredentialsUnavailable)?);
            if !(1..=MAX_CHUNKS).contains(&count) {
                return Err(SetupError::CredentialsUnavailable);
            }
            let mut chunks = vec![first];
            for index in 1..count {
                chunks.push(
                    credentials::read(&target(index))
                        .map_err(unavailable)?
                        .ok_or(SetupError::CredentialsUnavailable)?,
                );
            }
            join(&chunks)
                .map(Some)
                .ok_or(SetupError::CredentialsUnavailable)
        }
        fn write(&mut self, value: &[u8]) -> Result<(), SetupError> {
            let unavailable = |_| SetupError::CredentialsUnavailable;
            let chunks = split(value).ok_or(SetupError::CredentialsUnavailable)?;
            for (index, chunk) in chunks.iter().enumerate().rev() {
                credentials::write(&target(index), chunk).map_err(unavailable)?;
            }
            for index in chunks.len()..MAX_CHUNKS {
                credentials::delete(&target(index)).map_err(unavailable)?;
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn values_are_split_into_checked_chunks_and_joined_again() {
            for length in [0, 1, CHUNK, CHUNK + 1, 3 * CHUNK] {
                let value = (0..length).map(|n| n as u8).collect::<Vec<_>>();
                let chunks = split(&value).unwrap();
                assert!(chunks.iter().all(|chunk| chunk.len() <= MAX_BLOB));
                assert_eq!(join(&chunks).unwrap().as_slice(), value.as_slice());
            }
            assert!(split(&vec![0; MAX_CHUNKS * CHUNK + 1]).is_none());
        }
        #[test]
        fn mixed_generations_are_rejected() {
            let long = split(&vec![1; CHUNK + 1]).unwrap();
            let short = split(b"short").unwrap();
            assert!(join(&[long[0].clone(), short[0].clone()]).is_none());
            assert!(join(&long[..1]).is_none());
        }
        #[test]
        #[ignore = "uses the Credential Manager of the current Windows user"]
        fn native_blob_roundtrip_over_several_items() {
            let mut cache = VaultWifiCache(CredentialBlob);
            for index in 0..MAX_CHUNKS {
                credentials::delete(&target(index)).unwrap();
            }
            for number in 0..MAX_ENTRIES {
                cache
                    .remember(&format!("Network {number:02}"), &"p".repeat(63))
                    .unwrap();
            }
            assert_eq!(
                cache.cached_password("Network 00").unwrap().as_deref(),
                Some("p".repeat(63).as_str())
            );
            for index in 0..MAX_CHUNKS {
                credentials::delete(&target(index)).unwrap();
            }
            assert_eq!(cache.cached_password("Network 00").unwrap(), None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Memory(Option<Vec<u8>>);
    impl BlobVault for Memory {
        fn read(&mut self) -> Result<Option<Zeroizing<Vec<u8>>>, SetupError> {
            Ok(self.0.clone().map(Zeroizing::new))
        }
        fn write(&mut self, value: &[u8]) -> Result<(), SetupError> {
            self.0 = Some(value.to_vec());
            Ok(())
        }
    }

    #[test]
    fn remembers_replaces_and_finds_passwords_in_one_item() {
        let mut cache = VaultWifiCache(Memory::default());
        assert_eq!(cache.cached_password("Home").unwrap(), None);
        cache.remember("Home", "password-one").unwrap();
        cache.remember("Büro", "password-two").unwrap();
        cache.remember("Home", "password-three").unwrap();
        assert_eq!(
            cache.cached_password("Home").unwrap().as_deref(),
            Some("password-three")
        );
        assert_eq!(
            cache.cached_password("Büro").unwrap().as_deref(),
            Some("password-two")
        );
        assert_eq!(cache.cached_password("Other").unwrap(), None);
    }
    #[test]
    fn corrupt_items_are_unavailable_and_never_overwritten() {
        let mut cache = VaultWifiCache(Memory(Some(b"garbage".to_vec())));
        assert_eq!(
            cache.cached_password("Home"),
            Err(SetupError::CredentialsUnavailable)
        );
        assert_eq!(
            cache.remember("Home", "password-one"),
            Err(SetupError::CredentialsUnavailable)
        );
        assert_eq!(cache.0.0.as_deref(), Some(&b"garbage"[..]));
    }
    #[test]
    fn invalid_credentials_are_not_stored() {
        let mut cache = VaultWifiCache(Memory::default());
        assert_eq!(
            cache.remember("Home", "short"),
            Err(SetupError::InvalidInput)
        );
        assert!(cache.0.0.is_none());
    }
}
