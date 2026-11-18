use std::collections::BTreeMap;
use zorvinkel_core::{network::*, registry::*};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceState {
    pub registry: Registry,
    pub settings: NetworkSettings,
    pub operations: BTreeMap<OperationId, NetworkOperation>,
}
impl DeviceState {
    pub fn initial(network: NetworkConfiguration) -> Self {
        Self {
            registry: Registry::default(),
            settings: NetworkSettings {
                last_accepted: network,
                restart_policy: RestartPolicy::LastAccepted,
            },
            operations: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateError {
    Missing,
    Corrupt,
    UnsupportedVersion,
    Io,
    UnsupportedPlatform,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationError {
    Storage(StateError),
    Registry(RegistryError),
    Network(NetworkError),
    Busy,
    Conflict,
    HistoryFull,
    Trust(zorvinkel_core::trust::TrustError),
}
impl From<StateError> for ApplicationError {
    fn from(value: StateError) -> Self {
        Self::Storage(value)
    }
}
impl From<RegistryError> for ApplicationError {
    fn from(value: RegistryError) -> Self {
        Self::Registry(value)
    }
}
impl From<NetworkError> for ApplicationError {
    fn from(value: NetworkError) -> Self {
        Self::Network(value)
    }
}

/// Updates are serialized, atomic and durable. Failure must not publish partial changes.
/// A failed commit may have reached disk; callers must reload before retrying.
pub trait StateStore {
    fn read(&mut self) -> Result<DeviceState, StateError>;
    fn update<T>(
        &mut self,
        change: impl FnOnce(&mut DeviceState) -> Result<T, ApplicationError>,
    ) -> Result<T, ApplicationError>;
}

/// An opaque identity handle; no private key export is part of this contract.
/// Key generation and handshake use belong to the agreed security adapter.
pub trait IdentityStore {
    type Handle;
    fn load_identity(&mut self) -> Result<Self::Handle, StateError>;
    fn public_identity(&self, handle: &Self::Handle) -> Result<Vec<u8>, StateError>;
}

impl From<zorvinkel_core::trust::TrustError> for ApplicationError {
    fn from(value: zorvinkel_core::trust::TrustError) -> Self {
        Self::Trust(value)
    }
}
