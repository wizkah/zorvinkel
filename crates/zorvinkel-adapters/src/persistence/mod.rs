//! Unix device storage. Initialize only during explicit setup; open never creates storage.
mod codec;
pub mod identity;
mod protected;
mod record;
pub mod secret;
pub mod trust;

use protected::ProtectedStore;
use std::path::Path;
use zorvinkel_application::state::*;

pub struct FileStateStore(ProtectedStore);
impl FileStateStore {
    pub fn initialize(directory: &Path, state: DeviceState) -> Result<Self, StateError> {
        Ok(Self(ProtectedStore::initialize(
            directory,
            &codec::encode(&state)?,
        )?))
    }
    pub fn open(directory: &Path) -> Result<Self, StateError> {
        let store = Self(ProtectedStore::open(directory)?);
        codec::decode(&store.0.read()?)?;
        Ok(store)
    }
}
impl StateStore for FileStateStore {
    fn read(&mut self) -> Result<DeviceState, StateError> {
        codec::decode(&self.0.read()?)
    }
    fn update<T>(
        &mut self,
        change: impl FnOnce(&mut DeviceState) -> Result<T, ApplicationError>,
    ) -> Result<T, ApplicationError> {
        self.0.update(|bytes| {
            let mut state = codec::decode(bytes)?;
            let result = change(&mut state)?;
            Ok((codec::encode(&state)?, result))
        })
    }
}

pub mod catalog;

#[cfg(all(feature = "security", any(target_os = "macos", windows, test)))]
pub(crate) mod client_identity;

pub mod names;
