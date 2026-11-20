//! Keys of the terminal's easy login (ADR-033): one private SSH key per device, kept in the
//! client's data directory so the next login needs no password.
use std::path::PathBuf;
use zeroize::Zeroizing;

/// What easy login stored for a device.
pub struct EasyLogin {
    pub user: String,
    /// The private key in OpenSSH format.
    pub key: Zeroizing<String>,
}

pub struct EasyLoginStore {
    directory: Option<PathBuf>,
}
impl EasyLoginStore {
    const FOLDER: &'static str = "easy_login";

    pub fn in_directory(directory: &std::path::Path) -> Self {
        Self {
            directory: Some(directory.join(Self::FOLDER)),
        }
    }
    /// For runs that must not touch the client's data, such as the simulation.
    pub fn none() -> Self {
        Self { directory: None }
    }
    /// The file name is the hex form of the device handle, so no handle can leave the folder.
    fn file(&self, device: &str) -> Option<PathBuf> {
        let name: String = device.bytes().map(|byte| format!("{byte:02x}")).collect();
        (!name.is_empty()).then(|| self.directory.as_ref().map(|path| path.join(name)))?
    }
    pub fn load(&self, device: &str) -> Option<EasyLogin> {
        let text = Zeroizing::new(std::fs::read_to_string(self.file(device)?).ok()?);
        let (user, key) = text.split_once('\n')?;
        (!user.is_empty() && !key.trim().is_empty()).then(|| EasyLogin {
            user: user.to_owned(),
            key: Zeroizing::new(key.to_owned()),
        })
    }
    /// Returns `false` when the key could not be stored; the caller must not claim easy login.
    pub fn save(&self, device: &str, user: &str, key: &str) -> bool {
        let (Some(path), false) = (self.file(device), user.contains(['\n', '\r'])) else {
            return false;
        };
        let content = Zeroizing::new(format!("{user}\n{key}"));
        let written = path
            .parent()
            .is_some_and(|folder| std::fs::create_dir_all(folder).is_ok())
            && write_private(&path, content.as_bytes()).is_ok();
        if !written {
            eprintln!("Easy login key not saved.");
        }
        written
    }
    pub fn forget(&self, device: &str) {
        if let Some(path) = self.file(device) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Owner-only where the platform has file modes; elsewhere the user's data directory protects it.
fn write_private(path: &std::path::Path, content: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)?.write_all(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("zorvinkel-easy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn a_key_is_stored_per_device_and_forgotten_again() {
        let directory = directory("roundtrip");
        let store = EasyLoginStore::in_directory(&directory);
        assert!(store.load("a").is_none());
        assert!(store.save("a", "pi", "KEY\nLINES\n"));
        let found = store.load("a").unwrap();
        assert_eq!(found.user, "pi");
        assert_eq!(found.key.as_str(), "KEY\nLINES\n");
        assert!(store.load("b").is_none());
        store.forget("a");
        assert!(store.load("a").is_none());
        let _ = std::fs::remove_dir_all(&directory);
    }
    #[test]
    fn odd_handles_stay_inside_the_folder_and_the_simulation_stores_nothing() {
        let directory = directory("odd");
        let store = EasyLoginStore::in_directory(&directory);
        assert!(store.save("../../x", "pi", "KEY"));
        assert!(store.load("../../x").is_some());
        assert!(!store.save("a", "p\ni", "KEY"));
        let none = EasyLoginStore::none();
        assert!(!none.save("a", "pi", "KEY"));
        assert!(none.load("a").is_none());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
