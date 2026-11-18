//! A durable index prevents deleted device bindings from becoming new TOFU contacts.
use super::{protected::ProtectedStore, record, trust::FileDeviceTrustStore};
use std::path::Path;
use zorvinkel_application::state::{ApplicationError, StateError};
const HEADER: &[u8] = b"ZORVCAT\x01";

pub fn initialize(root: &Path) -> Result<(), StateError> {
    ProtectedStore::initialize(&root.join("devices"), &record::encode(HEADER, &[]))?;
    Ok(())
}
pub fn validate(root: &Path) -> Result<(), StateError> {
    let store = ProtectedStore::open(&root.join("devices"))?;
    decode(&store.read()?)?;
    Ok(())
}
pub fn select(root: &Path, handle: &str) -> Result<FileDeviceTrustStore, StateError> {
    if handle.is_empty() || handle.len() > 100 {
        return Err(StateError::Corrupt);
    }
    let key: String = handle.bytes().map(|b| format!("{b:02x}")).collect();
    let index = ProtectedStore::open(&root.join("devices"))?;
    let known = index
        .update(|bytes| {
            let mut keys = decode(bytes)?;
            let known = keys.contains(&key);
            if !known {
                if keys.len() >= 256 {
                    return Err(StateError::Io.into());
                }
                keys.push(key.clone());
            }
            Ok((record::encode(HEADER, keys.join("\n").as_bytes()), known))
        })
        .map_err(|error| match error {
            ApplicationError::Storage(error) => error,
            _ => StateError::Io,
        })?;
    let path = root.join(format!("device-{key}"));
    if known {
        FileDeviceTrustStore::open(&path)
    } else {
        FileDeviceTrustStore::initialize(&path)
    }
}
fn decode(bytes: &[u8]) -> Result<Vec<String>, StateError> {
    let payload = record::decode(HEADER, bytes)?;
    let value = std::str::from_utf8(payload).map_err(|_| StateError::Corrupt)?;
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<_> = value.split('\n').map(str::to_owned).collect();
    if keys.len() > 256
        || keys.iter().any(|key| {
            key.is_empty()
                || key.len() > 200
                || !key.len().is_multiple_of(2)
                || !key.bytes().all(|b| b.is_ascii_hexdigit())
        })
    {
        return Err(StateError::Corrupt);
    }
    Ok(keys)
}
