use crate::state::ApplicationError;
use zorvinkel_core::trust::{DeviceKey, DeviceTrust, TrustMode};

/// A store represents one saved device record. Changes are atomic and durable.
/// Missing or corrupt existing storage must not become an empty trust record.
pub trait DeviceTrustStore {
    fn update<T>(
        &mut self,
        change: impl FnOnce(&mut DeviceTrust) -> Result<T, ApplicationError>,
    ) -> Result<T, ApplicationError>;
}

pub fn configure_device(
    store: &mut impl DeviceTrustStore,
    key: DeviceKey,
) -> Result<(), ApplicationError> {
    store.update(|trust| Ok(trust.configure(key)?))
}

/// Call only after the handshake proves possession of the presented key.
/// Administrative requests are allowed only after this transaction commits.
pub fn verify_device(
    store: &mut impl DeviceTrustStore,
    presented: DeviceKey,
    mode: TrustMode,
) -> Result<(), ApplicationError> {
    store.update(|trust| Ok(trust.verify_or_bind(presented, mode)?))
}

/// Check a presented key before disclosing the client's identity, without creating a binding.
pub fn check_device(
    store: &mut impl DeviceTrustStore,
    presented: DeviceKey,
    mode: TrustMode,
) -> Result<(), ApplicationError> {
    store.update(|trust| {
        let mut candidate = trust.clone();
        Ok(candidate.verify_or_bind(presented, mode)?)
    })
}
