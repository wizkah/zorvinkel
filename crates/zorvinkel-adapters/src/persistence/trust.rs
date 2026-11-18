use super::{protected::ProtectedStore, record};
use std::path::Path;
use zorvinkel_application::{
    state::{ApplicationError, StateError},
    trust::DeviceTrustStore,
};
use zorvinkel_core::trust::{DeviceKey, DeviceTrust};

const HEADER: &[u8] = b"ZORVTR\x01";

/// One directory per saved device record. Selecting the record is a client responsibility.
pub struct FileDeviceTrustStore(ProtectedStore);
impl FileDeviceTrustStore {
    pub fn initialize(directory: &Path) -> Result<Self, StateError> {
        Ok(Self(ProtectedStore::initialize(
            directory,
            &record::encode(HEADER, &[]),
        )?))
    }
    pub fn open(directory: &Path) -> Result<Self, StateError> {
        let store = Self(ProtectedStore::open(directory)?);
        decode(&store.0.read()?)?;
        Ok(store)
    }
}
impl DeviceTrustStore for FileDeviceTrustStore {
    fn update<T>(
        &mut self,
        change: impl FnOnce(&mut DeviceTrust) -> Result<T, ApplicationError>,
    ) -> Result<T, ApplicationError> {
        self.0.update(|bytes| {
            let mut trust = decode(bytes)?;
            let result = change(&mut trust)?;
            let payload = trust.expected_key().map_or(&[][..], DeviceKey::as_bytes);
            Ok((record::encode(HEADER, payload), result))
        })
    }
}
fn decode(bytes: &[u8]) -> Result<DeviceTrust, StateError> {
    let payload = record::decode(HEADER, bytes)?;
    let mut trust = DeviceTrust::default();
    if !payload.is_empty() {
        trust
            .configure(DeviceKey::new(payload.to_vec()).map_err(|_| StateError::Corrupt)?)
            .map_err(|_| StateError::Corrupt)?;
    }
    Ok(trust)
}
