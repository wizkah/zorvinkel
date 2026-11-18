//! The device's own Wi-Fi: stored credentials and the link helper. wpa_supplicant provides the
//! radio side; addressing and DHCP need privileges the service does not have, so it only
//! creates request files in its runtime directory and root-owned systemd units (see
//! `packaging/linux`) do the rest.
use crate::persistence::secret::FileSecretStore;
use std::{
    fs::{self, OpenOptions},
    io::ErrorKind,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use zorvinkel_application::{
    setup::{AccessPointSettings, propose_access_point, validate_access_point},
    state::StateError,
};
use zorvinkel_core::network::NetworkError;

pub const UNIT_DIRECTORY: &str = "/etc/systemd/system";
const PATH_UNIT: &str = "zorvinkel-ap.path";
/// Maximum wait for the helper to bring the link up or down.
const LINK_TIMEOUT: Duration = Duration::from_secs(20);

/// Legacy plain file (0600) of earlier versions; read once for the migration, then removed.
pub fn credentials_path(root: &Path) -> PathBuf {
    root.join("access-point")
}
/// Protected store of the credentials of the device's own Wi-Fi (ADR-026).
pub fn credentials_directory(root: &Path) -> PathBuf {
    root.join("access-point-credentials")
}
/// Exists while the access point is meant to come back after a restart.
pub fn restart_path(root: &Path) -> PathBuf {
    root.join("access-point-restart")
}
/// The stored credentials are missing parts, damaged or not writable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CredentialsError;

const FORMAT: &str = "zorvinkel-access-point-v1";

fn encode(ssid: &str, password: &str) -> zeroize::Zeroizing<Vec<u8>> {
    zeroize::Zeroizing::new(format!("{FORMAT}\n{ssid}\n{password}\n").into_bytes())
}
fn decode(bytes: &[u8]) -> Result<AccessPointSettings, CredentialsError> {
    let text = zeroize::Zeroizing::new(
        std::str::from_utf8(bytes)
            .map_err(|_| CredentialsError)?
            .to_owned(),
    );
    let mut lines = text.split('\n');
    let (Some(FORMAT), Some(ssid), Some(password), Some(""), None) = (
        lines.next(),
        lines.next(),
        lines.next(),
        lines.next(),
        lines.next(),
    ) else {
        return Err(CredentialsError);
    };
    validate_access_point(ssid, password).map_err(|_| CredentialsError)?;
    Ok(AccessPointSettings {
        ssid: ssid.into(),
        password: password.into(),
    })
}

/// `None` while no credentials were stored; unreadable or invalid data is an error. Credentials
/// of the legacy plain file move into the protected store on first read.
pub fn read_credentials(root: &Path) -> Result<Option<AccessPointSettings>, CredentialsError> {
    let directory = credentials_directory(root);
    match FileSecretStore::open(&directory) {
        Ok(store) => {
            let settings = decode(&store.read().map_err(|_| CredentialsError)?)?;
            // A crash between copying and removing leaves the old file behind.
            let _ = fs::remove_file(credentials_path(root));
            Ok(Some(settings))
        }
        Err(StateError::Missing) if !directory.exists() => migrate_legacy(root),
        Err(_) => Err(CredentialsError),
    }
}
fn migrate_legacy(root: &Path) -> Result<Option<AccessPointSettings>, CredentialsError> {
    let text = match fs::read_to_string(credentials_path(root)) {
        Ok(text) => zeroize::Zeroizing::new(text),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(CredentialsError),
    };
    let mut lines = text.split('\n');
    let (Some(ssid), Some(password), Some(""), None) =
        (lines.next(), lines.next(), lines.next(), lines.next())
    else {
        return Err(CredentialsError);
    };
    validate_access_point(ssid, password).map_err(|_| CredentialsError)?;
    write_credentials(root, ssid, password)?;
    Ok(Some(AccessPointSettings {
        ssid: ssid.into(),
        password: password.into(),
    }))
}
/// Replaces the credentials atomically in the protected store.
pub fn write_credentials(root: &Path, ssid: &str, password: &str) -> Result<(), CredentialsError> {
    validate_access_point(ssid, password).map_err(|_| CredentialsError)?;
    let directory = credentials_directory(root);
    let content = encode(ssid, password);
    if directory.exists() {
        FileSecretStore::open(&directory)
            .and_then(|store| store.replace(&content))
            .map_err(|_| CredentialsError)?;
    } else {
        FileSecretStore::initialize(&directory, &content).map_err(|_| CredentialsError)?;
    }
    // The old plain file must not outlive the protected copy.
    match fs::remove_file(credentials_path(root)) {
        Err(error) if error.kind() != ErrorKind::NotFound => Err(CredentialsError),
        _ => Ok(()),
    }
}
/// Creates the credentials of a device that has none: `Zorvinkel-<hostname>` and a random
/// password of maximal length. Returns whether it created them; the access point is not
/// switched on. Without randomness there are no credentials, never guessable ones.
pub fn ensure_credentials(
    root: &Path,
    hostname: &str,
    fill: impl FnMut(&mut [u8]) -> bool,
) -> Result<bool, CredentialsError> {
    if read_credentials(root)?.is_some() {
        return Ok(false);
    }
    let proposal = propose_access_point(hostname, fill).ok_or(CredentialsError)?;
    write_credentials(root, &proposal.ssid, &proposal.password)?;
    Ok(true)
}

/// Addressing and DHCP for the access point. Both directions must be repeatable.
pub trait AccessPointLink: Send {
    fn up(&mut self) -> Result<(), NetworkError>;
    fn down(&mut self) -> Result<(), NetworkError>;
}

/// The feature is only offered where the installer enabled the helper.
pub fn installed(unit_directory: &Path) -> bool {
    unit_directory
        .join("multi-user.target.wants")
        .join(PATH_UNIT)
        .exists()
}

pub struct SystemAccessPointLink {
    directory: PathBuf,
    timeout: Duration,
}
impl SystemAccessPointLink {
    pub fn new(directory: &Path) -> Self {
        Self::with_timeout(directory, LINK_TIMEOUT)
    }
    fn with_timeout(directory: &Path, timeout: Duration) -> Self {
        Self {
            directory: directory.into(),
            timeout,
        }
    }
    fn file(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }
    fn wait(
        &self,
        done: impl Fn() -> Option<Result<(), NetworkError>>,
    ) -> Result<(), NetworkError> {
        let deadline = Instant::now() + self.timeout;
        loop {
            if let Some(result) = done() {
                return result;
            }
            if Instant::now() >= deadline {
                return Err(NetworkError::TimedOut);
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
}
impl AccessPointLink for SystemAccessPointLink {
    fn up(&mut self) -> Result<(), NetworkError> {
        let _ = fs::remove_file(self.file("ap-failed"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.file("ap-up"))
        {
            Ok(_) => (),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => (),
            Err(_) => return Err(NetworkError::Unavailable),
        }
        let result = self.wait(|| {
            if self.file("ap-failed").exists() {
                Some(Err(NetworkError::Rejected))
            } else {
                self.file("ap-ready").exists().then_some(Ok(()))
            }
        });
        if result.is_err() {
            let _ = fs::remove_file(self.file("ap-up"));
        }
        result
    }
    fn down(&mut self) -> Result<(), NetworkError> {
        match fs::remove_file(self.file("ap-up")) {
            Err(error) if error.kind() != ErrorKind::NotFound => {
                return Err(NetworkError::Unavailable);
            }
            _ => (),
        }
        self.wait(|| (!self.file("ap-ready").exists()).then_some(Ok(())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("zv-ap-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }
    #[test]
    fn credentials_roundtrip_in_the_protected_store_and_reject_damage() {
        use std::os::unix::fs::PermissionsExt;
        let dir = directory("credentials");
        assert!(read_credentials(&dir).unwrap().is_none());
        write_credentials(&dir, "Zorvinkel-Box", "password-ok").unwrap();
        let stored = read_credentials(&dir).unwrap().unwrap();
        assert_eq!(
            (stored.ssid.as_str(), stored.password.as_str()),
            ("Zorvinkel-Box", "password-ok")
        );
        write_credentials(&dir, "Zorvinkel-Neu", "password-neu").unwrap();
        assert_eq!(
            read_credentials(&dir).unwrap().unwrap().ssid,
            "Zorvinkel-Neu"
        );
        let store = credentials_directory(&dir);
        assert_eq!(
            fs::metadata(&store).unwrap().permissions().mode() & 0o077,
            0
        );
        assert_eq!(
            fs::metadata(store.join("state"))
                .unwrap()
                .permissions()
                .mode()
                & 0o077,
            0
        );
        assert!(!credentials_path(&dir).exists());
        assert!(write_credentials(&dir, "Box", "short").is_err());
        assert!(write_credentials(&dir, "Box\nX", "password-ok").is_err());
        for damaged in [
            "only-one-line\n",
            "zorvinkel-access-point-v1\na\npassword-ok",
            "zorvinkel-access-point-v1\na\nb\nc\n",
            "zorvinkel-access-point-v1\n\npassword-ok\n",
            "wrong-header\nBox\npassword-ok\n",
        ] {
            fs::write(store.join("state"), damaged).unwrap();
            assert!(read_credentials(&dir).is_err(), "{damaged:?}");
        }
        // Loose rights and an unexpected state never read as "no credentials yet".
        fs::set_permissions(store.join("state"), fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_credentials(&dir).is_err());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn legacy_plain_file_moves_into_the_protected_store() {
        let dir = directory("legacy");
        fs::write(credentials_path(&dir), "Zorvinkel-Alt\npassword-alt\n").unwrap();
        let stored = read_credentials(&dir).unwrap().unwrap();
        assert_eq!(
            (stored.ssid.as_str(), stored.password.as_str()),
            ("Zorvinkel-Alt", "password-alt")
        );
        assert!(!credentials_path(&dir).exists());
        assert_eq!(
            read_credentials(&dir).unwrap().unwrap().ssid,
            "Zorvinkel-Alt"
        );
        // A leftover plain file next to the protected store is dropped, the store wins.
        fs::write(credentials_path(&dir), "Fremd\npassword-fremd\n").unwrap();
        assert_eq!(
            read_credentials(&dir).unwrap().unwrap().ssid,
            "Zorvinkel-Alt"
        );
        assert!(!credentials_path(&dir).exists());
        // Damaged legacy data is an error and is kept for inspection.
        let broken = directory("legacy-broken");
        fs::write(credentials_path(&broken), "nur-eine-zeile\n").unwrap();
        assert!(read_credentials(&broken).is_err());
        assert!(credentials_path(&broken).exists());
        assert!(!credentials_directory(&broken).exists());
        fs::remove_dir_all(dir).unwrap();
        fs::remove_dir_all(broken).unwrap();
    }
    #[test]
    fn missing_credentials_are_generated_once_and_never_replaced() {
        let dir = directory("generate");
        let mut counter = 0_u8;
        let mut fill = |bytes: &mut [u8]| {
            for byte in bytes {
                *byte = counter;
                counter = counter.wrapping_add(7);
            }
            true
        };
        assert_eq!(ensure_credentials(&dir, "Fotobox", &mut fill), Ok(true));
        let first = read_credentials(&dir).unwrap().unwrap();
        assert_eq!(first.ssid, "Zorvinkel-Fotobox");
        assert_eq!(first.password.len(), 63);
        assert_eq!(ensure_credentials(&dir, "Fotobox", &mut fill), Ok(false));
        assert_eq!(
            read_credentials(&dir).unwrap().unwrap().password,
            first.password
        );
        // A damaged store is not silently replaced by new credentials.
        fs::write(credentials_directory(&dir).join("state"), "kaputt").unwrap();
        assert!(ensure_credentials(&dir, "Fotobox", fill).is_err());
        let fresh = directory("generate-no-random");
        assert_eq!(
            ensure_credentials(&fresh, "Box", |_| false),
            Err(CredentialsError)
        );
        assert!(read_credentials(&fresh).unwrap().is_none());
        fs::remove_dir_all(dir).unwrap();
        fs::remove_dir_all(fresh).unwrap();
    }
    #[test]
    fn link_waits_for_the_helper_and_reports_its_failure() {
        let dir = directory("link");
        let mut link = SystemAccessPointLink::with_timeout(&dir, Duration::from_secs(5));
        let helper_dir = dir.clone();
        let helper = thread::spawn(move || {
            while !helper_dir.join("ap-up").exists() {
                thread::sleep(Duration::from_millis(10));
            }
            fs::write(helper_dir.join("ap-ready"), b"").unwrap();
            while helper_dir.join("ap-up").exists() {
                thread::sleep(Duration::from_millis(10));
            }
            fs::remove_file(helper_dir.join("ap-ready")).unwrap();
        });
        link.up().unwrap();
        // Repeating either direction is harmless.
        link.up().unwrap();
        link.down().unwrap();
        link.down().unwrap();
        helper.join().unwrap();
        assert!(!dir.join("ap-up").exists());

        fs::write(dir.join("ap-failed"), b"").unwrap();
        let mut failing = SystemAccessPointLink::with_timeout(&dir, Duration::from_millis(300));
        // `up` clears a stale failure marker, so a silent helper ends in a timeout instead.
        assert_eq!(failing.up(), Err(NetworkError::TimedOut));
        assert!(!dir.join("ap-up").exists());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn helper_failure_is_rejected_and_cleans_up() {
        let dir = directory("failure");
        let mut link = SystemAccessPointLink::with_timeout(&dir, Duration::from_secs(5));
        let helper_dir = dir.clone();
        let helper = thread::spawn(move || {
            while !helper_dir.join("ap-up").exists() {
                thread::sleep(Duration::from_millis(10));
            }
            fs::write(helper_dir.join("ap-failed"), b"").unwrap();
        });
        assert_eq!(link.up(), Err(NetworkError::Rejected));
        helper.join().unwrap();
        assert!(!dir.join("ap-up").exists());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn helper_must_be_installed_for_the_feature_to_be_offered() {
        let dir = directory("units");
        assert!(!installed(&dir));
        let wants = dir.join("multi-user.target.wants");
        fs::create_dir_all(&wants).unwrap();
        fs::write(wants.join(PATH_UNIT), b"").unwrap();
        assert!(installed(&dir));
        fs::remove_dir_all(dir).unwrap();
    }
}
