//! Optional display metadata; it never creates or replaces a trust binding.
use super::{protected::ProtectedStore, record};
use std::path::Path;
use zorvinkel_application::{device_names::RegisteredName, state::StateError};
const HEADER: &[u8] = b"ZORVNAMES\x01";

pub fn read(root: &Path) -> Result<Vec<RegisteredName>, StateError> {
    let path = root.join("device-names");
    if !path.try_exists().map_err(|_| StateError::Io)? {
        return Ok(vec![]);
    }
    decode(&ProtectedStore::open(&path)?.read()?)
}
pub fn remember(root: &Path, name: RegisteredName) -> Result<(), StateError> {
    let path = root.join("device-names");
    if !path.try_exists().map_err(|_| StateError::Io)? {
        match ProtectedStore::initialize(&path, &record::encode(HEADER, &[])) {
            Ok(_) => (),
            Err(_) if path.exists() => {
                ProtectedStore::open(&path)?;
            }
            Err(error) => return Err(error),
        }
    }
    ProtectedStore::open(&path)?.update(|bytes| {
        let mut names = decode(bytes)?;
        names.retain(|entry| entry.handle != name.handle);
        names.push(name.clone());
        let bytes = encode(&names)?;
        Ok((bytes, ()))
    })
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn unhex(value: &str) -> Result<Vec<u8>, StateError> {
    if !value.len().is_multiple_of(2) || !value.is_ascii() {
        return Err(StateError::Corrupt);
    }
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(|_| StateError::Corrupt))
        .collect()
}
fn encode(names: &[RegisteredName]) -> Result<Vec<u8>, StateError> {
    if names.len() > 256
        || names.iter().any(|name| {
            name.handle.is_empty()
                || name.handle.len() > 100
                || name.hostname.is_empty()
                || name.hostname.len() > 100
                || name.hostname.chars().any(char::is_control)
        })
    {
        return Err(StateError::Corrupt);
    }
    let payload = names
        .iter()
        .map(|name| {
            format!(
                "{}\t{}\t{}",
                hex(&name.identity),
                hex(name.handle.as_bytes()),
                name.hostname
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(record::encode(HEADER, payload.as_bytes()))
}
fn decode(bytes: &[u8]) -> Result<Vec<RegisteredName>, StateError> {
    let payload = record::decode(HEADER, bytes)?;
    let payload = std::str::from_utf8(payload).map_err(|_| StateError::Corrupt)?;
    let mut names = Vec::new();
    if payload.is_empty() {
        return Ok(names);
    }
    for line in payload.split('\n') {
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 3 {
            return Err(StateError::Corrupt);
        }
        let identity = unhex(fields[0])?
            .try_into()
            .map_err(|_| StateError::Corrupt)?;
        let handle = String::from_utf8(unhex(fields[1])?).map_err(|_| StateError::Corrupt)?;
        if names
            .iter()
            .any(|name: &RegisteredName| name.handle == handle)
        {
            return Err(StateError::Corrupt);
        }
        names.push(RegisteredName {
            identity,
            handle,
            hostname: fields[2].into(),
        });
    }
    encode(&names)?;
    Ok(names)
}
