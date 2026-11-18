use super::selection::{NetworkStack, Selection, SystemProbe, select};
use super::status::{ObservedNetwork, observe};
use crate::{
    ble::{
        BleResult, STATUS,
        server::{ServerBackend, serve},
    },
    identity::IdentityHandle,
    persistence::FileStateStore,
    security::{
        ble::Responder,
        service::{ServiceTime, SessionRegistry},
        storage::load_identity,
    },
};
use std::{
    collections::BTreeMap,
    future::Future,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zorvinkel_application::PortError;

pub struct RuntimeNetwork {
    status: ObservedNetwork,
    wifi: Option<super::wifi::WifiController>,
    power: Option<super::power::SystemPower>,
    system: super::system::SystemReader,
    services: super::services::ServiceReader,
    service_control: Option<super::services::ServiceControlQueue>,
    ip_endpoint: bool,
}
impl zorvinkel_application::device::NetworkStatusSource for RuntimeNetwork {
    fn read_status(&mut self) -> Result<zorvinkel_core::NetworkStatus, PortError> {
        self.status.read_status()
    }
    fn system_info(&mut self) -> Option<&mut dyn zorvinkel_application::system::SystemInfoSource> {
        Some(&mut self.system)
    }
    fn service_info(
        &mut self,
    ) -> Option<&mut dyn zorvinkel_application::services::ServiceInfoSource> {
        self.services.configured().then_some(&mut self.services)
    }
    fn service_control(
        &mut self,
    ) -> Option<&mut dyn zorvinkel_application::services::ServiceControl> {
        self.service_control
            .as_mut()
            .map(|control| control as &mut dyn zorvinkel_application::services::ServiceControl)
    }
    fn ip_endpoint_available(&self) -> bool {
        self.ip_endpoint
    }
    fn power_control(&mut self) -> Option<&mut dyn zorvinkel_application::power::PowerControl> {
        self.power
            .as_mut()
            .map(|power| power as &mut dyn zorvinkel_application::power::PowerControl)
    }
    fn wifi_setup(&mut self) -> Option<&mut dyn zorvinkel_application::wifi::WifiSetupControl> {
        self.wifi
            .as_mut()
            .map(|wifi| wifi as &mut dyn zorvinkel_application::wifi::WifiSetupControl)
    }
}
pub struct RuntimeState {
    pub store: FileStateStore,
    pub sessions: SessionRegistry,
    pub network: RuntimeNetwork,
    identity: IdentityHandle,
    pub root: std::path::PathBuf,
    peers: BTreeMap<u64, Responder>,
    epoch: u64,
    started: std::time::Instant,
    /// Origin of the clock that the Bluetooth loop passes to `poll_timeouts`. IP sessions must
    /// use the same clock, or that poll would see them as expired or moving backwards.
    clock: std::time::Instant,
    /// Withdrawn when the state is dropped.
    announcement: Option<crate::discovery::Announcement>,
}
impl RuntimeState {
    pub fn open(root: &Path) -> BleResult<Self> {
        Ok(Self {
            store: FileStateStore::open(&root.join("registry"))
                .map_err(|_| "device state unavailable")?,
            sessions: SessionRegistry::default(),
            identity: load_identity(root)?,
            root: root.into(),
            network: RuntimeNetwork {
                status: ObservedNetwork(Err(PortError::Unavailable)),
                wifi: None,
                power: None,
                system: super::system::SystemReader::default(),
                services: super::services::ServiceReader::new(
                    std::path::Path::new(super::services::CONFIG_DIRECTORY),
                    false,
                ),
                service_control: None,
                ip_endpoint: false,
            },
            peers: BTreeMap::new(),
            epoch: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            started: std::time::Instant::now(),
            clock: std::time::Instant::now(),
            announcement: None,
        })
    }
    pub fn registration_seconds(&self) -> u64 {
        self.epoch.saturating_add(self.started.elapsed().as_secs())
    }
    pub fn public_key(&self) -> &[u8] {
        self.identity.public_key()
    }
    /// The time that the session registry and its timeout poll share.
    pub fn now(&self) -> Duration {
        self.clock.elapsed()
    }
    pub fn new_ip_responder(&self, now: Duration) -> BleResult<Responder> {
        Ok(Responder::new_ip(&self.identity, now)?)
    }
    /// Feeds one record of an IP connection to its responder.
    pub fn receive_ip(
        &mut self,
        responder: &mut Responder,
        record: &[u8],
        now: Duration,
    ) -> BleResult<Option<Vec<u8>>> {
        let time = ServiceTime {
            elapsed: now,
            registration_seconds: self.registration_seconds(),
        };
        Ok(responder.receive(
            record,
            &mut self.store,
            &mut self.network,
            time,
            &self.sessions,
        )?)
    }
}
pub struct NativeBackend(pub Arc<Mutex<RuntimeState>>);
impl ServerBackend for NativeBackend {
    /// The announcement is optional: clients still reach the device by a known address.
    fn ready(&mut self, advertised_name: &str, elapsed: Duration) {
        let Ok(mut state) = self.0.lock() else {
            return;
        };
        state.clock = std::time::Instant::now()
            .checked_sub(elapsed)
            .unwrap_or_else(std::time::Instant::now);
        if !state.network.ip_endpoint {
            return;
        }
        match crate::discovery::announce(advertised_name, state.public_key()) {
            Ok(announcement) => state.announcement = Some(announcement),
            Err(error) => eprintln!("mDNS announcement unavailable: {error}"),
        }
    }
    fn open(&mut self, connection: u64, now: Duration) -> BleResult<()> {
        let mut state = self.0.lock().map_err(|_| "runtime lock failed")?;
        if state.peers.len() >= 4 {
            return Err("session capacity reached".into());
        }
        let peer = Responder::new(&state.identity, now)?;
        state.peers.insert(connection, peer);
        Ok(())
    }
    fn receive(
        &mut self,
        connection: u64,
        message: &[u8],
        now: Duration,
    ) -> BleResult<Option<Vec<u8>>> {
        let mut state = self.0.lock().map_err(|_| "runtime lock failed")?;
        let registration_seconds = state.registration_seconds();
        let RuntimeState {
            peers,
            store,
            network,
            sessions,
            ..
        } = &mut *state;
        Ok(peers
            .get_mut(&connection)
            .ok_or("session missing")?
            .receive(
                message,
                store,
                network,
                ServiceTime {
                    elapsed: now,
                    registration_seconds,
                },
                sessions,
            )?)
    }
    fn poll(&mut self, now: Duration) -> BleResult<()> {
        self.0
            .lock()
            .map_err(|_| "runtime lock failed")?
            .sessions
            .poll_timeouts(now)?;
        Ok(())
    }
    fn closed(&self, connection: u64, now: Duration) -> bool {
        self.0.lock().map_or(true, |state| {
            state
                .peers
                .get(&connection)
                .is_none_or(|peer| peer.closed(now))
        })
    }
    fn remove(&mut self, connection: u64) {
        if let Ok(mut state) = self.0.lock() {
            state.peers.remove(&connection);
        }
    }
}

/// Where the status shown to clients comes from; it follows the selected stack.
enum StatusSource {
    WpaCli,
    #[cfg(feature = "network-manager")]
    NetworkManager(Arc<Mutex<super::network_manager::NetworkManagerStatus>>),
}
impl StatusSource {
    async fn read(
        &self,
        interface: &str,
        socket_directory: &Path,
    ) -> Result<zorvinkel_core::NetworkStatus, PortError> {
        match self {
            Self::WpaCli => observe(interface, socket_directory).await,
            #[cfg(feature = "network-manager")]
            Self::NetworkManager(status) => {
                let status = status.clone();
                tokio::task::spawn_blocking(move || {
                    status.lock().map_err(|_| PortError::Unavailable)?.read()
                })
                .await
                .map_err(|_| PortError::Unavailable)?
            }
        }
    }
}

pub async fn run(
    root: &Path,
    socket: &Path,
    interface: String,
    shutdown: impl Future<Output = ()>,
) -> BleResult<()> {
    let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")?;
    let hostname = hostname.trim();
    if hostname.is_empty() || hostname.chars().any(char::is_control) {
        return Err("device hostname unavailable".into());
    }
    let local_directory = socket
        .parent()
        .ok_or("admin socket requires a parent directory")?;
    let mut runtime = RuntimeState::open(root)?;
    if super::power::installed(Path::new(super::power::UNIT_DIRECTORY)) {
        runtime.network.power = Some(super::power::SystemPower::new(local_directory));
    }
    // Service actions need the root helper; without it the list stays display-only.
    if super::services::ServiceControlQueue::installed(Path::new(super::power::UNIT_DIRECTORY)) {
        let requests = local_directory.join(super::services::REQUEST_DIRECTORY);
        std::fs::create_dir_all(&requests)?;
        runtime.network.services =
            super::services::ServiceReader::new(Path::new(super::services::CONFIG_DIRECTORY), true);
        runtime.network.service_control = Some(super::services::ServiceControlQueue::new(
            Path::new(super::services::CONFIG_DIRECTORY),
            &requests,
        ));
    }
    // Only the selected stack gets a controller; a limitation leaves network setup
    // unavailable (reported through capabilities) without touching the network.
    let mut status_source = StatusSource::WpaCli;
    match select(&SystemProbe::default(), &interface) {
        Selection::Selected(NetworkStack::WpaDhcpcd) => {
            runtime.network.wifi = Some(
                super::wifi::WifiController::open(
                    &root.join("registry"),
                    &interface,
                    local_directory,
                    // Only where the installer enabled the helper for addressing and DHCP.
                    super::access_point::installed(Path::new(super::access_point::UNIT_DIRECTORY))
                        .then(|| {
                            Box::new(super::access_point::SystemAccessPointLink::new(
                                local_directory,
                            ))
                                as Box<dyn super::access_point::AccessPointLink>
                        }),
                )
                .map_err(|_| "Wi-Fi worker unavailable")?,
            );
        }
        Selection::Selected(NetworkStack::NetworkManager) => {
            #[cfg(feature = "network-manager")]
            {
                runtime.network.wifi = Some(
                    super::wifi::WifiController::open_network_manager(
                        &root.join("registry"),
                        &interface,
                    )
                    .map_err(|_| "Wi-Fi worker unavailable")?,
                );
                status_source = StatusSource::NetworkManager(Arc::new(Mutex::new(
                    super::network_manager::NetworkManagerStatus::new(&interface),
                )));
            }
            #[cfg(not(feature = "network-manager"))]
            eprintln!(
                "Network setup unavailable: {:?}",
                super::selection::Limitation::NetworkManagerUnsupported
            );
        }
        Selection::Limited(limitation) => {
            eprintln!("Network setup unavailable: {limitation:?}");
        }
    }
    // Without the endpoint the device simply does not offer the reachability test.
    let ip_listener = match super::ip::bind().await {
        Ok(listener) => {
            runtime.network.ip_endpoint = true;
            Some(listener)
        }
        Err(error) => {
            eprintln!("IP reachability endpoint unavailable: {error}");
            None
        }
    };
    let state = Arc::new(Mutex::new(runtime));
    let listener = super::admin::bind(socket)?;
    let observe_state = state.clone();
    let socket_directory = socket
        .parent()
        .ok_or("admin socket requires a parent directory")?
        .to_owned();
    let status = async move {
        loop {
            let result = status_source.read(&interface, &socket_directory).await;
            observe_state
                .lock()
                .map_err(|_| "runtime lock failed")?
                .network
                .status
                .0 = result;
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        #[allow(unreachable_code)]
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    };
    let ip_state = state.clone();
    let ip_server = async move {
        match ip_listener {
            Some(listener) => super::ip::serve(listener, ip_state).await,
            None => std::future::pending().await,
        }
    };
    let result = tokio::select! {
        result = ip_server => result,
        result = serve(STATUS, hostname, NativeBackend(state.clone()), shutdown) => result,
        result = super::admin::serve(listener, state) => result,
        result = status => result,
    };
    let _ = std::fs::remove_file(socket);
    result
}
