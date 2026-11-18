//! Binding rules. Presented keys must originate from a verified cryptographic handshake.
use crate::registry::ClientKey;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceKey(ClientKey);
impl DeviceKey {
    pub fn new(bytes: Vec<u8>) -> Result<Self, TrustError> {
        Ok(Self(
            ClientKey::new(bytes).map_err(|_| TrustError::InvalidKey)?,
        ))
    }
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustMode {
    TrustOnFirstUse,
    RequireKnownKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrustError {
    InvalidKey,
    UnknownDevice,
    KeyMismatch,
}

/// One client-side device record, selected before connecting. Discovery names are not bindings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceTrust {
    expected: Option<DeviceKey>,
}
impl DeviceTrust {
    pub fn expected_key(&self) -> Option<&DeviceKey> {
        self.expected.as_ref()
    }

    /// Setup never silently replaces an existing binding.
    pub fn configure(&mut self, key: DeviceKey) -> Result<(), TrustError> {
        self.verify_or_bind(key, TrustMode::TrustOnFirstUse)
    }

    pub fn verify_or_bind(
        &mut self,
        presented: DeviceKey,
        mode: TrustMode,
    ) -> Result<(), TrustError> {
        match &self.expected {
            Some(expected) if expected == &presented => Ok(()),
            Some(_) => Err(TrustError::KeyMismatch),
            None if mode == TrustMode::RequireKnownKey => Err(TrustError::UnknownDevice),
            None => {
                self.expected = Some(presented);
                Ok(())
            }
        }
    }
}
