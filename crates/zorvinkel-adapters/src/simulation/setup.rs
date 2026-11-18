use super::SimulatedNetworkControl;
use crate::memory::MemoryStateStore;
use zorvinkel_application::{
    network::{execute, set_restart_policy},
    registration::register,
    setup::*,
    state::{DeviceState, StateStore},
};
use zorvinkel_core::{
    DeviceStatus, DiscoveredDevice, IpReachability, NetworkStatus, network::*, registry::ClientKey,
};

pub const DEMO_WIFI_PASSWORD: &str = "demo-wlan-2026";
pub const DEMO_AP_PASSWORD: &str = "demo-geraet-2026";

/// In-memory fixture. It never reads host credentials or changes networking.
pub struct SimulatedSetup {
    store: MemoryStateStore,
    network: SimulatedNetworkControl,
    devices: Vec<DiscoveredDevice>,
    connected: bool,
    discovered: bool,
    scanned: bool,
    wifi_persistence: bool,
    /// What the simulated device stored; `None` makes the setup propose defaults.
    access_point: Option<AccessPointSettings>,
    next_operation: u64,
    services: Vec<zorvinkel_application::services::ServiceEntry>,
}
impl Default for SimulatedSetup {
    fn default() -> Self {
        Self::with_devices(vec![DiscoveredDevice {
            handle: "simulation:device-1".into(),
            display_name: "Photo booth · Demo".into(),
        }])
    }
}
impl SimulatedSetup {
    /// A fleet of `count` numbered devices, for exercising the UI under load.
    pub fn with_device_count(count: usize) -> Self {
        Self::with_devices(
            (1..=count)
                .map(|number| DiscoveredDevice {
                    handle: format!("simulation:device-{number}"),
                    display_name: format!("Photo booth {number:02}"),
                })
                .collect(),
        )
    }
    /// Skips the search step, as if the devices had already been found.
    pub fn discovered(mut self) -> Self {
        self.discovered = true;
        self
    }
    fn with_devices(devices: Vec<DiscoveredDevice>) -> Self {
        Self {
            devices,
            store: MemoryStateStore::new(DeviceState::initial(NetworkConfiguration::Disconnected)),
            network: SimulatedNetworkControl {
                configuration: NetworkConfiguration::Disconnected,
                apply_error: None,
                restore_error: None,
                apply_count: 0,
            },
            connected: false,
            discovered: false,
            scanned: false,
            wifi_persistence: false,
            access_point: None,
            next_operation: 1,
            services: simulated_services(),
        }
    }
}
impl SetupBackend for SimulatedSetup {
    fn handle(&mut self, request: SetupRequest) -> Result<SetupUpdate, SetupError> {
        let error = self.apply(request).err();
        Ok(SetupUpdate {
            snapshot: self.snapshot()?,
            error,
            saved_password: None,
        })
    }
}
impl SimulatedSetup {
    fn proposed_or_stored(&self) -> AccessPointSettings {
        self.access_point
            .clone()
            .unwrap_or_else(|| AccessPointSettings {
                ssid: default_access_point_ssid("Demo"),
                password: DEMO_AP_PASSWORD.into(),
            })
    }
    fn actor() -> ClientKey {
        ClientKey::new(vec![1; 32]).expect("valid synthetic public key")
    }

    fn apply(&mut self, request: SetupRequest) -> Result<(), SetupError> {
        match request {
            SetupRequest::Discover => {
                self.discovered = true;
                return Ok(());
            }
            SetupRequest::Connect(handle) => {
                if !self.discovered || !self.devices.iter().any(|device| device.handle == handle) {
                    return Err(SetupError::Disconnected);
                }
                register(&mut self.store, Self::actor(), 0).map_err(|_| SetupError::Unavailable)?;
                self.connected = true;
                self.scanned = true;
                return Ok(());
            }
            SetupRequest::Disconnect => {
                self.connected = false;
                self.scanned = false;
                return Ok(());
            }
            SetupRequest::Refresh => return Ok(()),
            _ => (),
        }
        if !self.connected {
            return Err(SetupError::Disconnected);
        }
        match request {
            SetupRequest::Scan => {
                self.scanned = true;
                Ok(())
            }
            SetupRequest::ConnectWifi { ssid, password } => {
                validate_wifi(&ssid, &password)?;
                self.network.apply_error =
                    (password != DEMO_WIFI_PASSWORD).then_some(NetworkError::Rejected);
                let result = self.change(NetworkIntent::ConnectWifi(
                    ProfileId::new(ssid).map_err(|_| SetupError::InvalidInput)?,
                ));
                self.network.apply_error = None;
                result
            }
            SetupRequest::EnableAccessPoint(settings) => {
                validate_access_point(&settings.ssid, &settings.password)?;
                self.access_point = Some(settings);
                self.change(NetworkIntent::EnableAccessPoint(
                    ProfileId::new("demo-ap".into()).expect("valid fixture profile"),
                ))
            }
            SetupRequest::DisableAccessPoint => self.change(NetworkIntent::DisableAccessPoint),
            SetupRequest::UpdateAccessPoint(settings) => {
                validate_access_point(&settings.ssid, &settings.password)?;
                self.access_point = Some(settings);
                Ok(())
            }
            SetupRequest::RestartLastAccepted => {
                set_restart_policy(&mut self.store, &Self::actor(), RestartPolicy::LastAccepted)
                    .map_err(|_| SetupError::Unavailable)
            }
            SetupRequest::RestartCurrentAsDefault => {
                if self.network.configuration == NetworkConfiguration::Disconnected {
                    return Err(SetupError::NoActiveNetwork);
                }
                set_restart_policy(
                    &mut self.store,
                    &Self::actor(),
                    RestartPolicy::DefaultProfile(self.network.configuration.clone()),
                )
                .map_err(|_| SetupError::Unavailable)
            }
            SetupRequest::SetWifiPersistence(enabled) => {
                self.wifi_persistence = enabled;
                // Choosing between keeping and the original settings ends a fixed default.
                set_restart_policy(&mut self.store, &Self::actor(), RestartPolicy::LastAccepted)
                    .map_err(|_| SetupError::Unavailable)
            }
            SetupRequest::ServiceAction { name, action } => {
                use zorvinkel_application::services::{ServiceAction, ServiceState};
                let service = self
                    .services
                    .iter_mut()
                    .find(|service| service.name == name && service.actions.contains(&action))
                    .ok_or(SetupError::Unsupported)?;
                service.state = match action {
                    ServiceAction::Start | ServiceAction::Restart => ServiceState::Active,
                    ServiceAction::Stop => ServiceState::Inactive,
                };
                Ok(())
            }
            SetupRequest::PowerDevice(_) => {
                // The simulated device vanishes like a real one; nothing on the host changes.
                self.connected = false;
                self.scanned = false;
                Ok(())
            }
            SetupRequest::ReadSavedCredentials { .. }
            | SetupRequest::ReadCachedCredentials { .. } => Err(SetupError::CredentialsUnavailable),
            _ => unreachable!("connection commands were handled before authorization"),
        }
    }

    fn change(&mut self, intent: NetworkIntent) -> Result<(), SetupError> {
        let id = OperationId(self.next_operation);
        self.next_operation = self
            .next_operation
            .checked_add(1)
            .ok_or(SetupError::Unavailable)?;
        let operation = execute(
            &mut self.store,
            &mut self.network,
            &Self::actor(),
            id,
            intent,
        )
        .map_err(|_| SetupError::Unavailable)?;
        if matches!(operation.status, OperationStatus::Succeeded { .. }) {
            Ok(())
        } else {
            Err(SetupError::ConnectionRejected)
        }
    }

    fn snapshot(&mut self) -> Result<SetupSnapshot, SetupError> {
        let devices = if self.discovered {
            self.devices.clone()
        } else {
            Vec::new()
        };
        if !self.connected {
            return Ok(SetupSnapshot {
                devices,
                ..Default::default()
            });
        }
        let network = match &self.network.configuration {
            NetworkConfiguration::Disconnected => NetworkStatus::Disconnected,
            NetworkConfiguration::Wifi(profile) => NetworkStatus::WifiConnected {
                ssid: profile.as_str().into(),
            },
            NetworkConfiguration::AccessPoint(_) => NetworkStatus::AccessPointActive {
                ssid: self.proposed_or_stored().ssid,
            },
        };
        Ok(SetupSnapshot {
            devices,
            status: Some(DeviceStatus {
                network,
                ip_reachability: IpReachability::Unverified,
            }),
            networks: if self.scanned {
                vec!["Event-WiFi".into(), "Studio".into(), "Guest network".into()]
            } else {
                Vec::new()
            },
            wifi_available: true,
            power_available: true,
            system_info_available: true,
            system_info: Some(zorvinkel_application::system::SystemInfo {
                version: env!("CARGO_PKG_VERSION").into(),
                uptime_seconds: 93_784,
                load_milli: [120, 340, 280],
                memory_total: 485 * 1024 * 1024,
                memory_available: 310 * 1024 * 1024,
                storage_total: 14 * 1024 * 1024 * 1024,
                storage_free: 9 * 1024 * 1024 * 1024,
                temperature_milli: Some(47_800),
                cpu_mhz: Some(700),
                cpu_max_mhz: Some(1000),
                throttle_flags: Some(0),
                os_name: Some("Raspbian GNU/Linux 10 (buster)".into()),
                kernel: Some("5.10.103+".into()),
                model: Some("Raspberry Pi Zero W Rev 1.1".into()),
                hostname: Some("photobooth".into()),
                addresses: vec![zorvinkel_application::system::NetworkAddress {
                    interface: "wlan0".into(),
                    address: std::net::Ipv4Addr::new(192, 168, 1, 42).into(),
                }],
            }),
            services_available: true,
            services: self.services.clone(),
            service_actions_available: true,
            wifi_persistence: Some(self.wifi_persistence),
            access_point_available: true,
            access_point: Some(self.proposed_or_stored()),
            access_point_proposed: self.access_point.is_none(),
            restart_policy: Some(
                self.store
                    .read()
                    .map_err(|_| SetupError::Unavailable)?
                    .settings
                    .restart_policy,
            ),
            transport: None,
            session_remaining: None,
        })
    }
}

fn simulated_services() -> Vec<zorvinkel_application::services::ServiceEntry> {
    use zorvinkel_application::services::{
        ServiceAction, ServiceEntry, ServiceState, WebLink, WebScheme,
    };
    let all = vec![
        ServiceAction::Start,
        ServiceAction::Stop,
        ServiceAction::Restart,
    ];
    vec![
        ServiceEntry {
            group: "Music".into(),
            name: "Raspotify".into(),
            version: Some("0.48.1".into()),
            state: ServiceState::Active,
            actions: all.clone(),
            web: None,
        },
        ServiceEntry {
            group: "Music".into(),
            name: "Mopidy".into(),
            version: Some("3.4.2".into()),
            state: ServiceState::Inactive,
            actions: all.clone(),
            web: Some(WebLink {
                label: Some("Web interface".into()),
                scheme: WebScheme::Http,
                port: 6680,
                path: "/iris/".into(),
            }),
        },
        ServiceEntry {
            group: "Photo booth".into(),
            name: "Photo booth app".into(),
            version: None,
            state: ServiceState::Failed,
            actions: vec![ServiceAction::Start],
            web: None,
        },
    ]
}
