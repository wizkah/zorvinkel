use zorvinkel_adapters::simulation::{SimulatedDiscovery, SimulatedNetwork, SimulatedStatusSource};
use zorvinkel_application::{PortError, client::Client};
use zorvinkel_core::{DiscoveredDevice, IpReachability, NetworkStatus};

#[test]
fn discovered_device_reports_network_without_claiming_ip_reachability() {
    let network = SimulatedNetwork {
        status: Ok(NetworkStatus::WifiConnected {
            ssid: "Test network".into(),
        }),
    };
    let mut client = Client::new(SimulatedDiscovery, SimulatedStatusSource::new(network));
    let devices = client.discover().unwrap();
    assert_eq!(devices.len(), 1);
    let status = client.read_status(&devices[0]).unwrap();
    assert_eq!(
        status.network,
        NetworkStatus::WifiConnected {
            ssid: "Test network".into()
        }
    );
    assert_eq!(status.ip_reachability, IpReachability::Unverified);
}

#[test]
fn network_failures_reach_the_client() {
    let network = SimulatedNetwork {
        status: Err(PortError::Unavailable),
    };
    let mut client = Client::new(SimulatedDiscovery, SimulatedStatusSource::new(network));
    let devices = client.discover().unwrap();
    assert_eq!(client.read_status(&devices[0]), Err(PortError::Unavailable));
}

#[test]
fn unknown_discovery_handle_does_not_select_another_device() {
    let mut client = Client::new(
        SimulatedDiscovery,
        SimulatedStatusSource::new(SimulatedNetwork::default()),
    );
    let unknown = DiscoveredDevice {
        handle: "unknown".into(),
        display_name: "Simulated Zorvinkel".into(),
        ..Default::default()
    };
    assert_eq!(client.read_status(&unknown), Err(PortError::Disconnected));
}

#[test]
fn discovery_failure_is_not_reported_as_an_empty_list() {
    struct FailedDiscovery;
    impl zorvinkel_application::client::DeviceDiscovery for FailedDiscovery {
        fn discover(&mut self) -> Result<Vec<DiscoveredDevice>, PortError> {
            Err(PortError::Unavailable)
        }
    }
    let mut client = Client::new(
        FailedDiscovery,
        SimulatedStatusSource::new(SimulatedNetwork::default()),
    );
    assert_eq!(client.discover(), Err(PortError::Unavailable));
}
