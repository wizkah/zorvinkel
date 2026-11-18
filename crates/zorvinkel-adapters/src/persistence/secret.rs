//! Protected storage for one small secret that the service creates itself, such as the
//! credentials of the device's own Wi-Fi. It shares directory rights (0700), file rights (0600),
//! locking and atomic replacement with the identity and state stores (ADR-003); the content is
//! not encrypted on the disk. Opening never creates anything and damaged content is an error.
use super::protected::ProtectedStore;
use std::path::Path;
use zeroize::Zeroizing;
use zorvinkel_application::state::StateError;

pub struct FileSecretStore(ProtectedStore);
impl FileSecretStore {
    /// The directory must not exist yet.
    pub fn initialize(directory: &Path, secret: &[u8]) -> Result<Self, StateError> {
        Ok(Self(ProtectedStore::initialize(directory, secret)?))
    }
    pub fn open(directory: &Path) -> Result<Self, StateError> {
        Ok(Self(ProtectedStore::open(directory)?))
    }
    pub fn read(&self) -> Result<Zeroizing<Vec<u8>>, StateError> {
        self.0.read()
    }
    pub fn replace(&self, secret: &[u8]) -> Result<(), StateError> {
        self.0
            .update::<_, StateError>(|_| Ok((secret.to_vec(), ())))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn directory(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("zv-secret-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }
    #[test]
    fn secret_is_private_replaceable_and_never_created_by_open() {
        let dir = directory("roundtrip");
        assert_eq!(FileSecretStore::open(&dir).err(), Some(StateError::Missing));
        let store = FileSecretStore::initialize(&dir, b"first").unwrap();
        assert_eq!(store.read().unwrap().as_slice(), b"first");
        store.replace(b"second").unwrap();
        assert_eq!(
            FileSecretStore::open(&dir)
                .unwrap()
                .read()
                .unwrap()
                .as_slice(),
            b"second"
        );
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o077,
            0
        );
        assert_eq!(
            std::fs::metadata(dir.join("state"))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
        // A second initialization must not replace an existing secret.
        assert!(FileSecretStore::initialize(&dir, b"other").is_err());
        assert_eq!(store.read().unwrap().as_slice(), b"second");
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn loose_rights_and_missing_state_are_errors() {
        let dir = directory("rights");
        let store = FileSecretStore::initialize(&dir, b"secret").unwrap();
        std::fs::set_permissions(dir.join("state"), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert!(store.read().is_err());
        assert!(FileSecretStore::open(&dir).is_err());
        std::fs::set_permissions(dir.join("state"), std::fs::Permissions::from_mode(0o600))
            .unwrap();
        std::fs::remove_file(dir.join("state")).unwrap();
        assert!(FileSecretStore::open(&dir).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
