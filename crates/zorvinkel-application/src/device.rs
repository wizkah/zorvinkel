use crate::PortError;
use zorvinkel_core::NetworkStatus;

pub trait NetworkStatusSource {
    fn wifi_setup(&mut self) -> Option<&mut dyn crate::wifi::WifiSetupControl> {
        None
    }
    fn power_control(&mut self) -> Option<&mut dyn crate::power::PowerControl> {
        None
    }
    fn system_info(&mut self) -> Option<&mut dyn crate::system::SystemInfoSource> {
        None
    }
    fn service_info(&mut self) -> Option<&mut dyn crate::services::ServiceInfoSource> {
        None
    }
    fn service_control(&mut self) -> Option<&mut dyn crate::services::ServiceControl> {
        None
    }
    /// The device accepts authenticated IP connections for the reachability test.
    fn ip_endpoint_available(&self) -> bool {
        false
    }
    fn read_status(&mut self) -> Result<NetworkStatus, PortError>;
}

/// Local observation only; transport entry points must enforce authorization.
pub struct Device<N> {
    network: N,
}

impl<N: NetworkStatusSource> Device<N> {
    pub fn new(network: N) -> Self {
        Self { network }
    }

    pub fn network_status(&mut self) -> Result<NetworkStatus, PortError> {
        self.network.read_status()
    }
}
