//! Client identity references and restart-safe migration into an OS secret store.
use super::{identity, protected::ProtectedStore, record};
use crate::{
    identity::{IdentityHandle, IdentityMaterial},
    security::identity::validate,
};
use std::path::Path;
use zeroize::Zeroizing;
use zorvinkel_application::state::StateError;

const REFERENCE_HEADER: &[u8] = b"ZORVKC\x01";

pub(crate) trait IdentityVault {
    /// Missing is distinct from denied access; implementations never overwrite an item.
    fn read(&mut self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StateError>;
    fn insert(&mut self, account: &str, value: &[u8]) -> Result<(), StateError>;
}

pub(crate) fn initialize(
    directory: &Path,
    material: IdentityMaterial,
    vault: &mut impl IdentityVault,
) -> Result<IdentityHandle, StateError> {
    let handle = IdentityHandle::from_material(material);
    validate(&handle).map_err(|_| StateError::Corrupt)?;
    let store = ProtectedStore::initialize(directory, &reference(handle.public_key()))?;
    // A failed first installation remains incomplete, rather than generating a new identity.
    store.update(|_| {
        save_verified(&handle, vault)?;
        Ok::<_, StateError>((reference(handle.public_key()), ()))
    })?;
    Ok(handle)
}

pub(crate) fn open(
    directory: &Path,
    vault: &mut impl IdentityVault,
) -> Result<IdentityHandle, StateError> {
    let store = ProtectedStore::open(directory)?;
    store.update(|bytes| {
        if bytes.starts_with(REFERENCE_HEADER) {
            let public = record::decode(REFERENCE_HEADER, bytes)?;
            if public.len() != 32 {
                return Err(StateError::Corrupt);
            }
            let value = vault.read(&account(public))?.ok_or(StateError::Missing)?;
            let handle = decode_identity(&value)?;
            if handle.public_key() != public {
                return Err(StateError::Corrupt);
            }
            return Ok((reference(public), handle));
        }
        let handle = decode_identity(bytes)?;
        save_verified(&handle, vault)?;
        // Commit the reference only after rereading the identical identity from the vault.
        Ok((reference(handle.public_key()), handle))
    })
}
fn save_verified(
    handle: &IdentityHandle,
    vault: &mut impl IdentityVault,
) -> Result<(), StateError> {
    let account = account(handle.public_key());
    let value = identity::encode(&handle.material);
    if vault.read(&account)?.is_none() {
        // A competing migration may have inserted this exact item in another profile.
        if let Err(error) = vault.insert(&account, &value)
            && vault.read(&account)?.is_none()
        {
            return Err(error);
        }
    }
    let saved = vault.read(&account)?.ok_or(StateError::Missing)?;
    if saved.as_slice() != value.as_slice() {
        return Err(StateError::Corrupt);
    }
    Ok(())
}
fn decode_identity(value: &[u8]) -> Result<IdentityHandle, StateError> {
    let handle = IdentityHandle::from_material(identity::decode(value)?);
    validate(&handle).map_err(|_| StateError::Corrupt)?;
    Ok(handle)
}
fn account(public: &[u8]) -> String {
    public.iter().map(|b| format!("{b:02x}")).collect()
}
fn reference(public: &[u8]) -> Vec<u8> {
    record::encode(REFERENCE_HEADER, public)
}

#[cfg(target_os = "macos")]
pub(crate) mod macos {
    use super::*;
    use security_framework::os::macos::keychain::SecKeychain;
    use security_framework_sys::base::errSecItemNotFound;
    const SERVICE: &str = "org.zorvinkel.client.identity";

    pub(crate) struct KeychainVault;
    impl IdentityVault for KeychainVault {
        fn read(&mut self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StateError> {
            let keychain = SecKeychain::default().map_err(|_| StateError::Io)?;
            match keychain.find_generic_password(SERVICE, account) {
                Ok((password, _)) => Ok(Some(Zeroizing::new(password.to_vec()))),
                Err(error) if error.code() == errSecItemNotFound => Ok(None),
                Err(_) => Err(StateError::Io),
            }
        }
        fn insert(&mut self, account: &str, value: &[u8]) -> Result<(), StateError> {
            SecKeychain::default()
                .map_err(|_| StateError::Io)?
                .add_generic_password(SERVICE, account, value)
                .map_err(|_| StateError::Io)
        }
    }
}

#[cfg(windows)]
pub(crate) mod windows {
    use super::*;
    use crate::windows_credentials as credentials;
    const SERVICE: &str = "org.zorvinkel.client.identity";

    pub(crate) struct CredentialVault;
    impl IdentityVault for CredentialVault {
        fn read(&mut self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StateError> {
            credentials::read(&format!("{SERVICE}/{account}")).map_err(|_| StateError::Io)
        }
        fn insert(&mut self, account: &str, value: &[u8]) -> Result<(), StateError> {
            // The system call replaces silently, so refuse an existing item here.
            if self.read(account)?.is_some() {
                return Err(StateError::Io);
            }
            credentials::write(&format!("{SERVICE}/{account}"), value).map_err(|_| StateError::Io)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::{
            persistence::identity::FileIdentityStore, security::identity::generate_identity,
        };

        #[test]
        #[ignore = "uses the Credential Manager of the current Windows user"]
        fn native_credential_migration_preserves_identity_and_never_overwrites() {
            let directory = std::env::temp_dir().join(format!("zv-cred-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&directory);
            let material = generate_identity().unwrap();
            let public = material.public.clone();
            FileIdentityStore::initialize(&directory, material).unwrap();
            let mut vault = CredentialVault;
            assert_eq!(open(&directory, &mut vault).unwrap().public_key(), public);
            assert_eq!(open(&directory, &mut vault).unwrap().public_key(), public);
            assert!(vault.insert(&account(&public), b"other").is_err());
            credentials::delete(&format!("{SERVICE}/{}", account(&public))).unwrap();
            assert_eq!(
                open(&directory, &mut vault).err(),
                Some(StateError::Missing)
            );
            std::fs::remove_dir_all(directory).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{persistence::identity::FileIdentityStore, security::identity::generate_identity};
    use std::{collections::BTreeMap, fs};

    #[derive(Default)]
    struct Vault {
        items: BTreeMap<String, Zeroizing<Vec<u8>>>,
        denied: bool,
        fail_insert: bool,
        fail_readback: bool,
    }
    impl IdentityVault for Vault {
        fn read(&mut self, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, StateError> {
            if self.denied || (self.fail_readback && !self.items.is_empty()) {
                return Err(StateError::Io);
            }
            Ok(self
                .items
                .get(account)
                .map(|value| Zeroizing::new(value.to_vec())))
        }
        fn insert(&mut self, account: &str, value: &[u8]) -> Result<(), StateError> {
            if self.fail_insert || self.items.contains_key(account) {
                return Err(StateError::Io);
            }
            self.items
                .insert(account.into(), Zeroizing::new(value.to_vec()));
            Ok(())
        }
    }
    struct Directory(std::path::PathBuf);
    impl Directory {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let name = format!(
                "zorvinkel-keychain-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            Self(std::env::temp_dir().join(name))
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            if self.0.exists() {
                fs::remove_dir_all(&self.0).unwrap();
            }
        }
    }
    fn legacy(directory: &Path) -> Vec<u8> {
        let material = generate_identity().unwrap();
        let public = material.public.clone();
        FileIdentityStore::initialize(directory, material).unwrap();
        public
    }
    #[test]
    fn migration_preserves_identity_and_replaces_only_the_private_record() {
        let directory = Directory::new();
        let public = legacy(&directory.0);
        let old = Zeroizing::new(fs::read(directory.0.join("state")).unwrap());
        let mut vault = Vault::default();
        assert_eq!(open(&directory.0, &mut vault).unwrap().public_key(), public);
        let saved = fs::read(directory.0.join("state")).unwrap();
        assert_eq!(record::decode(REFERENCE_HEADER, &saved).unwrap(), public);
        assert_eq!(vault.items[&account(&public)].as_slice(), old.as_slice());
        assert_eq!(open(&directory.0, &mut vault).unwrap().public_key(), public);
        assert!(FileIdentityStore::open(&directory.0).is_err());
    }
    #[test]
    fn denied_insert_or_readback_preserves_the_legacy_file_and_allows_retry() {
        for failure in 0..3 {
            let directory = Directory::new();
            let public = legacy(&directory.0);
            let old = Zeroizing::new(fs::read(directory.0.join("state")).unwrap());
            let mut vault = Vault {
                denied: failure == 0,
                fail_insert: failure == 1,
                fail_readback: failure == 2,
                ..Default::default()
            };
            assert!(open(&directory.0, &mut vault).is_err());
            assert_eq!(fs::read(directory.0.join("state")).unwrap(), old.as_slice());
            vault.denied = false;
            vault.fail_insert = false;
            vault.fail_readback = false;
            assert_eq!(open(&directory.0, &mut vault).unwrap().public_key(), public);
        }
    }
    #[test]
    fn missing_denied_or_wrong_keychain_entry_never_creates_an_identity() {
        let directory = Directory::new();
        let mut vault = Vault::default();
        let identity = initialize(&directory.0, generate_identity().unwrap(), &mut vault).unwrap();
        let public = identity.public_key().to_vec();
        let saved = fs::read(directory.0.join("state")).unwrap();
        vault.denied = true;
        assert!(open(&directory.0, &mut vault).is_err());
        vault.denied = false;
        vault.items.clear();
        assert_eq!(
            open(&directory.0, &mut vault).err(),
            Some(StateError::Missing)
        );
        assert!(vault.items.is_empty());
        let other = generate_identity().unwrap();
        vault
            .items
            .insert(account(&public), identity::encode(&other));
        assert_eq!(
            open(&directory.0, &mut vault).err(),
            Some(StateError::Corrupt)
        );
        assert_eq!(fs::read(directory.0.join("state")).unwrap(), saved);
    }
    #[test]
    fn interrupted_migration_reuses_existing_exact_keychain_item() {
        let directory = Directory::new();
        let public = legacy(&directory.0);
        let old = Zeroizing::new(fs::read(directory.0.join("state")).unwrap());
        let mut vault = Vault::default();
        vault.items.insert(account(&public), old);
        vault.fail_insert = true;
        assert_eq!(open(&directory.0, &mut vault).unwrap().public_key(), public);
        assert_eq!(vault.items.len(), 1);
    }
    #[test]
    fn conflicting_existing_item_is_not_overwritten() {
        let directory = Directory::new();
        let public = legacy(&directory.0);
        let old = Zeroizing::new(fs::read(directory.0.join("state")).unwrap());
        let other = identity::encode(&generate_identity().unwrap());
        let mut vault = Vault::default();
        vault.items.insert(account(&public), other.clone());
        assert_eq!(
            open(&directory.0, &mut vault).err(),
            Some(StateError::Corrupt)
        );
        assert_eq!(vault.items[&account(&public)].as_slice(), other.as_slice());
        assert_eq!(fs::read(directory.0.join("state")).unwrap(), old.as_slice());
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "requires access to the local macOS keychain"]
    fn native_keychain_migration_preserves_identity_and_reads_after_reopen() {
        use security_framework::os::macos::keychain::SecKeychain;
        // Do not block an unattended test on a keychain permission dialog.
        let _interaction = SecKeychain::disable_user_interaction().unwrap();
        let directory = Directory::new();
        let public = legacy(&directory.0);
        let mut vault = macos::KeychainVault;
        assert_eq!(open(&directory.0, &mut vault).unwrap().public_key(), public);
        assert_eq!(open(&directory.0, &mut vault).unwrap().public_key(), public);
        let keychain = SecKeychain::default().unwrap();
        let (_, item) = keychain
            .find_generic_password("org.zorvinkel.client.identity", &account(&public))
            .unwrap();
        item.delete();
        assert_eq!(
            open(&directory.0, &mut vault).err(),
            Some(StateError::Missing)
        );
    }
    #[test]
    fn failed_first_installation_is_not_automatically_reinitialized() {
        let directory = Directory::new();
        let mut vault = Vault {
            fail_insert: true,
            ..Default::default()
        };
        assert!(initialize(&directory.0, generate_identity().unwrap(), &mut vault).is_err());
        let saved = fs::read(directory.0.join("state")).unwrap();
        assert_eq!(
            open(&directory.0, &mut vault).err(),
            Some(StateError::Missing)
        );
        assert!(initialize(&directory.0, generate_identity().unwrap(), &mut vault).is_err());
        assert_eq!(fs::read(directory.0.join("state")).unwrap(), saved);
    }
}
