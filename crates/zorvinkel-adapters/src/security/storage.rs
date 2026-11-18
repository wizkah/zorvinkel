//! Explicit first installation; opening an existing installation never repairs missing files.
use super::{
    SecureError,
    identity::{generate_identity, validate},
};
use crate::{
    identity::IdentityHandle,
    persistence::{FileStateStore, identity::FileIdentityStore},
};
use std::path::Path;
use zorvinkel_application::state::{DeviceState, IdentityStore, StateError};
use zorvinkel_core::network::NetworkConfiguration;

pub fn initialize_device(root: &Path) -> Result<(), SecureError> {
    create_root(root)?;
    FileIdentityStore::initialize(&root.join("identity"), generate_identity()?)
        .map_err(storage_error)?;
    FileStateStore::initialize(
        &root.join("registry"),
        DeviceState::initial(NetworkConfiguration::Disconnected),
    )
    .map_err(storage_error)?;
    Ok(())
}
pub fn create_root(root: &Path) -> Result<(), SecureError> {
    create_private_directory(root).map_err(|_| SecureError::Storage)
}
pub fn load_identity(root: &Path) -> Result<IdentityHandle, SecureError> {
    let identity = FileIdentityStore::open(&root.join("identity"))
        .map_err(storage_error)?
        .load_identity()
        .map_err(storage_error)?;
    validate(&identity)?;
    Ok(identity)
}
pub fn storage_error(_: StateError) -> SecureError {
    SecureError::Storage
}

/// Native client storage is platform-specific; device storage remains unchanged.
pub fn initialize_client_identity(root: &Path) -> Result<(), SecureError> {
    #[cfg(target_os = "macos")]
    {
        use crate::persistence::client_identity::{self, macos::KeychainVault};
        client_identity::initialize(
            &root.join("identity"),
            generate_identity()?,
            &mut KeychainVault,
        )
        .map_err(storage_error)?;
    }
    #[cfg(windows)]
    {
        use crate::persistence::client_identity::{self, windows::CredentialVault};
        client_identity::initialize(
            &root.join("identity"),
            generate_identity()?,
            &mut CredentialVault,
        )
        .map_err(storage_error)?;
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    FileIdentityStore::initialize(&root.join("identity"), generate_identity()?)
        .map_err(storage_error)?;
    Ok(())
}
pub fn load_client_identity(root: &Path) -> Result<IdentityHandle, SecureError> {
    #[cfg(target_os = "macos")]
    {
        use crate::persistence::client_identity::{self, macos::KeychainVault};
        client_identity::open(&root.join("identity"), &mut KeychainVault).map_err(storage_error)
    }
    #[cfg(windows)]
    {
        use crate::persistence::client_identity::{self, windows::CredentialVault};
        client_identity::open(&root.join("identity"), &mut CredentialVault).map_err(storage_error)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    load_identity(root)
}

#[cfg(unix)]
fn create_private_directory(root: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(root)
}
/// The root lies below the user profile, which Windows restricts to the user.
#[cfg(windows)]
fn create_private_directory(root: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new().create(root)
}
