use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;
use zorvinkel_application::state::StateError;

pub(super) struct ProtectedStore {
    directory: PathBuf,
}

impl ProtectedStore {
    pub fn initialize(directory: &Path, bytes: &[u8]) -> Result<Self, StateError> {
        platform::create_directory(directory).map_err(|_| StateError::Io)?;
        let store = Self {
            directory: directory.to_owned(),
        };
        platform::create_file(&directory.join("lock"))
            .map_err(|_| StateError::Io)?
            .sync_all()
            .map_err(|_| StateError::Io)?;
        store.write(bytes)?;
        if let Some(parent) = directory.parent() {
            platform::sync_directory(parent).map_err(|_| StateError::Io)?;
        }
        Ok(store)
    }

    pub fn open(directory: &Path) -> Result<Self, StateError> {
        let store = Self {
            directory: directory.to_owned(),
        };
        let _lock = store.lock()?;
        store.load()?;
        Ok(store)
    }

    fn lock(&self) -> Result<File, StateError> {
        let metadata = fs::symlink_metadata(&self.directory).map_err(map_io)?;
        if !metadata.is_dir() || !platform::is_private(&metadata) {
            return Err(StateError::Io);
        }
        let path = self.directory.join("lock");
        check_file(&path)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(map_io)?;
        file.lock().map_err(|_| StateError::Io)?;
        Ok(file)
    }

    fn load(&self) -> Result<Zeroizing<Vec<u8>>, StateError> {
        let path = self.directory.join("state");
        check_file(&path)?;
        let mut bytes = Zeroizing::new(Vec::new());
        File::open(path)
            .map_err(map_io)?
            .take(1_048_577)
            .read_to_end(&mut bytes)
            .map_err(map_io)?;
        if bytes.len() > 1_048_576 {
            return Err(StateError::Corrupt);
        }
        Ok(bytes)
    }

    fn write(&self, bytes: &[u8]) -> Result<(), StateError> {
        let pending = self.directory.join("next");
        match fs::remove_file(&pending) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(_) => return Err(StateError::Io),
        }
        let mut file = platform::create_file(&pending).map_err(map_io)?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(map_io)?;
        fs::rename(pending, self.directory.join("state")).map_err(map_io)?;
        platform::sync_directory(&self.directory).map_err(map_io)
    }
}

impl ProtectedStore {
    pub fn read(&self) -> Result<Zeroizing<Vec<u8>>, StateError> {
        let _lock = self.lock()?;
        self.load()
    }

    pub fn update<T, E: From<StateError>>(
        &self,
        change: impl FnOnce(&[u8]) -> Result<(Vec<u8>, T), E>,
    ) -> Result<T, E> {
        let _lock = self.lock()?;
        let bytes = self.load()?;
        let (next, result) = change(&bytes)?;
        if next.len() > 1_048_576 {
            return Err(StateError::Corrupt.into());
        }
        self.write(&next)?;
        Ok(result)
    }
}
fn check_file(path: &Path) -> Result<(), StateError> {
    let metadata = fs::symlink_metadata(path).map_err(map_io)?;
    if !metadata.is_file() || !platform::is_private(&metadata) {
        return Err(StateError::Io);
    }
    Ok(())
}
fn map_io(error: std::io::Error) -> StateError {
    if error.kind() == std::io::ErrorKind::NotFound {
        StateError::Missing
    } else {
        StateError::Io
    }
}

/// Owner-only rights. Unix enforces and checks modes; on Windows the storage lives below the
/// user profile, whose ACL already restricts access to the user, so no extra check exists.
#[cfg(unix)]
mod platform {
    use std::{
        fs::{DirBuilder, File, Metadata, OpenOptions},
        io,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
        path::Path,
    };
    pub fn create_directory(path: &Path) -> io::Result<()> {
        DirBuilder::new().mode(0o700).create(path)
    }
    pub fn create_file(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }
    pub fn is_private(metadata: &Metadata) -> bool {
        metadata.permissions().mode() & 0o077 == 0
    }
    pub fn sync_directory(path: &Path) -> io::Result<()> {
        File::open(path).and_then(|file| file.sync_all())
    }
}

#[cfg(windows)]
mod platform {
    use std::{
        fs::{DirBuilder, File, Metadata, OpenOptions},
        io,
        path::Path,
    };
    pub fn create_directory(path: &Path) -> io::Result<()> {
        DirBuilder::new().create(path)
    }
    pub fn create_file(path: &Path) -> io::Result<File> {
        OpenOptions::new().write(true).create_new(true).open(path)
    }
    pub fn is_private(_: &Metadata) -> bool {
        true
    }
    /// Windows cannot open directories as files.
    pub fn sync_directory(_: &Path) -> io::Result<()> {
        Ok(())
    }
}
