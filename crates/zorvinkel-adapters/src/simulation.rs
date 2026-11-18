use zorvinkel_application::{
    PortError,
    client::{DeviceDiscovery, DeviceStatusSource},
    device::{Device, NetworkStatusSource},
};
use zorvinkel_core::{DeviceStatus, DiscoveredDevice, IpReachability, NetworkStatus};

pub struct SimulatedDiscovery;

impl DeviceDiscovery for SimulatedDiscovery {
    fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, PortError> {
        Ok(vec![simulated_device()])
    }
}

fn simulated_device() -> DiscoveredDevice {
    DiscoveredDevice {
        handle: "simulation:device-1".into(),
        display_name: "Simulated Zorvinkel".into(),
    }
}

pub struct SimulatedNetwork {
    pub status: Result<NetworkStatus, PortError>,
}

impl Default for SimulatedNetwork {
    fn default() -> Self {
        Self {
            status: Ok(NetworkStatus::Disconnected),
        }
    }
}

impl NetworkStatusSource for SimulatedNetwork {
    fn read_status(&mut self) -> Result<NetworkStatus, PortError> {
        self.status.clone()
    }
}

/// In-process fixture only; it provides no authentication or transport.
pub struct SimulatedStatusSource {
    device: Device<SimulatedNetwork>,
}

impl SimulatedStatusSource {
    pub fn new(network: SimulatedNetwork) -> Self {
        Self {
            device: Device::new(network),
        }
    }
}

impl DeviceStatusSource for SimulatedStatusSource {
    fn read_status(&mut self, device: &DiscoveredDevice) -> Result<DeviceStatus, PortError> {
        if device.handle != simulated_device().handle {
            return Err(PortError::Disconnected);
        }
        Ok(DeviceStatus {
            network: self.device.network_status()?,
            ip_reachability: IpReachability::Unverified,
        })
    }
}

pub struct SimulatedNetworkControl {
    pub configuration: zorvinkel_core::network::NetworkConfiguration,
    pub apply_error: Option<zorvinkel_core::network::NetworkError>,
    pub restore_error: Option<zorvinkel_core::network::NetworkError>,
    pub apply_count: usize,
}

impl zorvinkel_application::network::NetworkControl for SimulatedNetworkControl {
    fn check_support(
        &mut self,
        _: &zorvinkel_core::network::NetworkIntent,
    ) -> Result<(), zorvinkel_core::network::NetworkError> {
        Ok(())
    }
    fn observe(
        &mut self,
    ) -> Result<zorvinkel_core::network::NetworkConfiguration, zorvinkel_core::network::NetworkError>
    {
        Ok(self.configuration.clone())
    }
    fn apply(
        &mut self,
        intent: &zorvinkel_core::network::NetworkIntent,
    ) -> Result<(), zorvinkel_core::network::NetworkError> {
        self.apply_count += 1;
        self.configuration = intent.target(&self.configuration);
        match self.apply_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
    fn restore(
        &mut self,
        previous: &zorvinkel_core::network::NetworkConfiguration,
    ) -> Result<(), zorvinkel_core::network::NetworkError> {
        if let Some(error) = self.restore_error {
            return Err(error);
        }
        self.configuration = previous.clone();
        Ok(())
    }
}

pub mod setup;
