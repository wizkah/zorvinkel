use crate::PortError;
use zorvinkel_core::{DeviceStatus, DiscoveredDevice};

pub trait DeviceDiscovery {
    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, PortError>;
}

/// Implementations must authenticate and authorize before returning device status.
pub trait DeviceStatusSource {
    fn read_status(&mut self, device: &DiscoveredDevice) -> Result<DeviceStatus, PortError>;
}

pub struct Client<D, S> {
    discovery: D,
    status_source: S,
}

impl<D: DeviceDiscovery, S: DeviceStatusSource> Client<D, S> {
    pub fn new(discovery: D, status_source: S) -> Self {
        Self {
            discovery,
            status_source,
        }
    }

    pub fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, PortError> {
        self.discovery.discover()
    }

    pub fn read_status(&mut self, device: &DiscoveredDevice) -> Result<DeviceStatus, PortError> {
        self.status_source.read_status(device)
    }
}
