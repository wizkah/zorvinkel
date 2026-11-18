use std::sync::{Arc, Mutex};
use zorvinkel_application::state::*;

#[derive(Clone)]
pub struct MemoryStateStore(Arc<Mutex<DeviceState>>);
impl MemoryStateStore {
    pub fn new(state: DeviceState) -> Self {
        Self(Arc::new(Mutex::new(state)))
    }
}
impl StateStore for MemoryStateStore {
    fn read(&mut self) -> Result<DeviceState, StateError> {
        self.0
            .lock()
            .map(|state| state.clone())
            .map_err(|_| StateError::Io)
    }
    fn update<T>(
        &mut self,
        change: impl FnOnce(&mut DeviceState) -> Result<T, ApplicationError>,
    ) -> Result<T, ApplicationError> {
        let mut state = self.0.lock().map_err(|_| StateError::Io)?;
        let mut next = state.clone();
        let result = change(&mut next)?;
        *state = next;
        Ok(result)
    }
}

#[derive(Clone, Default)]
pub struct MemoryDeviceTrustStore(Arc<Mutex<zorvinkel_core::trust::DeviceTrust>>);
impl zorvinkel_application::trust::DeviceTrustStore for MemoryDeviceTrustStore {
    fn update<T>(
        &mut self,
        change: impl FnOnce(&mut zorvinkel_core::trust::DeviceTrust) -> Result<T, ApplicationError>,
    ) -> Result<T, ApplicationError> {
        let mut current = self.0.lock().map_err(|_| StateError::Io)?;
        let mut next = current.clone();
        let result = change(&mut next)?;
        *current = next;
        Ok(result)
    }
}
