//! Protected storage for key material supplied by a cryptographic adapter.
//! Key generation and validation of the public/private relationship are not performed here.
use super::{protected::ProtectedStore, record};
use std::path::Path;
use zorvinkel_application::state::{IdentityStore, StateError};

const HEADER: &[u8] = b"ZORVID\x01";

pub use crate::identity::{IdentityHandle, IdentityMaterial};
use zeroize::Zeroizing;

pub struct FileIdentityStore(ProtectedStore);
impl FileIdentityStore {
    /// Only explicit setup creates an identity; reopening never replaces it.
    pub fn initialize(directory: &Path, material: IdentityMaterial) -> Result<Self, StateError> {
        let bytes = encode(&material);
        Ok(Self(ProtectedStore::initialize(directory, &bytes)?))
    }
    pub fn open(directory: &Path) -> Result<Self, StateError> {
        let store = Self(ProtectedStore::open(directory)?);
        decode(&store.0.read()?)?;
        Ok(store)
    }
}
impl IdentityStore for FileIdentityStore {
    type Handle = IdentityHandle;
    fn load_identity(&mut self) -> Result<IdentityHandle, StateError> {
        Ok(IdentityHandle {
            material: decode(&self.0.read()?)?,
        })
    }
    fn public_identity(&self, handle: &IdentityHandle) -> Result<Vec<u8>, StateError> {
        Ok(handle.material.public.clone())
    }
}

pub(super) fn encode(material: &IdentityMaterial) -> Zeroizing<Vec<u8>> {
    let mut payload = Zeroizing::new((material.public.len() as u32).to_le_bytes().to_vec());
    payload.extend((material.private.len() as u32).to_le_bytes());
    payload.extend(&material.public);
    payload.extend(&material.private);
    Zeroizing::new(record::encode(HEADER, &payload))
}
pub(super) fn decode(bytes: &[u8]) -> Result<IdentityMaterial, StateError> {
    let payload = record::decode(HEADER, bytes)?;
    let lengths = payload.get(..8).ok_or(StateError::Corrupt)?;
    let public_length =
        u32::from_le_bytes(lengths[..4].try_into().map_err(|_| StateError::Corrupt)?) as usize;
    let private_length =
        u32::from_le_bytes(lengths[4..].try_into().map_err(|_| StateError::Corrupt)?) as usize;
    if public_length > 1024
        || private_length > 4096
        || payload.len() != 8 + public_length + private_length
    {
        return Err(StateError::Corrupt);
    }
    IdentityMaterial::new(
        payload[8..8 + public_length].to_vec(),
        payload[8 + public_length..].to_vec(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_material_roundtrips_without_an_export_api() {
        let original = IdentityMaterial::new(vec![1; 32], vec![2; 32]).unwrap();
        let encoded = encode(&original);
        let restored = decode(&encoded).unwrap();
        assert_eq!(restored.public, original.public);
        assert_eq!(restored.private, original.private);
        for length in 0..encoded.len() {
            assert!(decode(&encoded[..length]).is_err());
        }
        for index in 0..encoded.len() {
            let mut damaged = encoded.clone();
            damaged[index] ^= 1;
            assert!(decode(&damaged).is_err());
        }
    }
}
