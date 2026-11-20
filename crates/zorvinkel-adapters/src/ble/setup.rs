//! Native desktop worker. Authenticated status and volatile Wi-Fi setup.
use super::{BleResult, STATUS, client::ProbeCentral, secure_client::StatusConnection};
use crate::{
    identity::IdentityHandle,
    persistence::catalog,
    security::storage::{create_root, initialize_client_identity, load_client_identity},
};
use btleplug::api::Peripheral as _;
use std::{
    collections::BTreeMap,
    net::IpAddr,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::{runtime::Runtime, time::timeout};
use zorvinkel_application::setup::*;
use zorvinkel_core::{DeviceStatus, IpReachability, NetworkStatus, trust::DeviceKey};
use zorvinkel_protocol::status::{Operation, ResponseBody};

pub struct NativeSetup {
    runtime: Runtime,
    central: Option<ProbeCentral>,
    identity: IdentityHandle,
    root: PathBuf,
    connection: Option<StatusConnection>,
    snapshot: SetupSnapshot,
    connected_handle: Option<String>,
    pending: BTreeMap<String, u64>,
    advertised_names: BTreeMap<String, String>,
    /// Pinned identity of the connected device; an IP peer must prove it.
    device_key: Option<DeviceKey>,
    /// Devices announced on the local network; a connection prefers this path over Bluetooth.
    ip_devices: BTreeMap<String, crate::discovery::IpDevice>,
    ip_endpoint: bool,
    /// The device ties a Bluetooth-to-IP switch to the running session with a one-time proof.
    correlation: bool,
    /// Skips the network path once, so a lost IP session continues over Bluetooth.
    force_ble: bool,
    reachability: Option<CachedReachability>,
}
/// A probe costs a handshake per address, so a recent result stands in for the next refreshes.
struct CachedReachability {
    addresses: Vec<IpAddr>,
    checked: Instant,
    result: IpReachability,
}
const REACHABILITY_TTL: Duration = Duration::from_secs(30);
const IP_SEARCH: Duration = Duration::from_secs(4);
const SERVICE_ACTION_SETTLE: Duration = Duration::from_secs(1);
impl NativeSetup {
    pub fn open(root: &Path) -> BleResult<Self> {
        if !root.try_exists()? {
            create_root(root)?;
            initialize_client_identity(root)?;
            catalog::initialize(root).map_err(|_| "device catalog initialization failed")?;
        }
        catalog::validate(root).map_err(|_| "device catalog unavailable")?;
        Ok(Self {
            runtime: Runtime::new()?,
            central: None,
            identity: load_client_identity(root)?,
            root: root.into(),
            connection: None,
            snapshot: SetupSnapshot::default(),
            connected_handle: None,
            pending: BTreeMap::new(),
            advertised_names: BTreeMap::new(),
            device_key: None,
            ip_devices: BTreeMap::new(),
            ip_endpoint: false,
            correlation: false,
            force_ble: false,
            reachability: None,
        })
    }
    pub fn public_key(&self) -> Vec<u8> {
        self.identity.public_key().to_vec()
    }
    /// The next connection skips the network path and uses Bluetooth, the only carrier of a tunnel.
    pub fn force_bluetooth(&mut self) {
        self.force_ble = true;
    }
    /// Opens a terminal tunnel over the connected Bluetooth device and returns the client end of
    /// the stream; SSH runs over it. It lives on this backend's runtime and closes when dropped.
    pub fn open_tunnel(&mut self) -> BleResult<tokio::io::DuplexStream> {
        let connection = self.connection.as_mut().ok_or("not connected")?;
        self.runtime.block_on(connection.open_tunnel())
    }
    /// Diagnostic for the tunnel (ADR-031): opens one, reads the identification line `sshd` sends
    /// first, answers with one and reads the start of the key exchange. Needs no SSH login.
    pub fn probe_tunnel(&mut self) -> BleResult<String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut stream = self.open_tunnel()?;
        self.runtime.block_on(async move {
            let started = Instant::now();
            let limit = Duration::from_secs(15);
            let mut banner = Vec::new();
            while !banner.ends_with(
                b"
",
            ) && banner.len() < 255
            {
                let mut byte = [0];
                timeout(limit, stream.read_exact(&mut byte)).await??;
                banner.push(byte[0]);
            }
            let first = started.elapsed();
            stream
                .write_all(
                    b"SSH-2.0-zorvinkel-probe
",
                )
                .await?;
            let mut exchange = vec![0; 512];
            timeout(limit, stream.read_exact(&mut exchange)).await??;
            Ok(format!(
                "identification {:?} after {first:?}; {} more bytes after {:?}",
                String::from_utf8_lossy(&banner).trim_end(),
                exchange.len(),
                started.elapsed()
            ))
        })
    }
    fn apply(&mut self, request: SetupRequest) -> BleResult<()> {
        self.runtime.block_on(async {
            match request {
                SetupRequest::Discover => {
                    if let Some(mut connection) = self.connection.take() {
                        connection.close().await;
                    }
                    reset_device(&mut self.snapshot);
                    self.connected_handle = None;
                    // Bluetooth and the local network are searched together; either may be missing.
                    let bluetooth = async {
                        if self.central.is_none() {
                            self.central = Some(ProbeCentral::open_profile(STATUS).await?);
                        }
                        self.central
                            .as_mut()
                            .ok_or("Bluetooth unavailable")?
                            .discover()
                            .await
                    };
                    let network =
                        tokio::task::spawn_blocking(|| crate::discovery::browse(IP_SEARCH));
                    let (bluetooth, network) = tokio::join!(bluetooth, network);
                    let announced = network.ok().and_then(Result::ok).unwrap_or_default();
                    self.snapshot.devices = match bluetooth {
                        Ok(devices) => devices,
                        Err(error) if announced.is_empty() => return Err(error),
                        Err(_) => Vec::new(),
                    };
                    self.ip_devices.clear();
                    let registered =
                        crate::persistence::names::read(&self.root).unwrap_or_default();
                    let bluetooth_names: Vec<&str> = self
                        .snapshot
                        .devices
                        .iter()
                        .map(|device| device.display_name.as_str())
                        .collect();
                    let assigned =
                        crate::discovery::assign_handles(announced, &registered, &bluetooth_names);
                    for (handle, device) in assigned {
                        // The instance equals the Bluetooth name but is always present, while a
                        // scan may not have delivered the name yet.
                        match self
                            .snapshot
                            .devices
                            .iter_mut()
                            .find(|known| known.handle == handle)
                        {
                            Some(known) => {
                                known.display_name = device.instance.clone();
                                known.via_ip = true;
                            }
                            None => self
                                .snapshot
                                .devices
                                .push(zorvinkel_core::DiscoveredDevice {
                                    handle: handle.clone(),
                                    display_name: device.instance.clone(),
                                    via_ble: false,
                                    via_ip: true,
                                }),
                        }
                        self.ip_devices.insert(handle, device);
                    }
                    self.advertised_names = self
                        .snapshot
                        .devices
                        .iter()
                        .map(|device| (device.handle.clone(), device.display_name.clone()))
                        .collect();
                    refresh_names(&self.root, &mut self.snapshot.devices);
                }
                SetupRequest::Connect(handle) => {
                    if let Some(mut connection) = self.connection.take() {
                        connection.close().await;
                    }
                    reset_device(&mut self.snapshot);
                    self.connected_handle = None;
                    let mut trust = catalog::select(&self.root, &handle)
                        .map_err(|_| "device binding unavailable")?;
                    let mut connection = open_connection(
                        self.central.as_ref(),
                        self.ip_devices.get(&handle).filter(|_| !self.force_ble),
                        &self.identity,
                        &handle,
                        &mut trust,
                    )
                    .await?;
                    // Register is idempotent for a known client, including a lost initial answer.
                    if connection.request(Operation::Register).await? != ResponseBody::Registered {
                        connection.close().await;
                        return Err(crate::security::SecureError::Unauthorized.into());
                    }
                    if connection.request(Operation::Authenticate).await?
                        != ResponseBody::Authenticated
                    {
                        connection.close().await;
                        return Err(crate::security::SecureError::Unauthorized.into());
                    }
                    let ResponseBody::Capabilities(bits) =
                        connection.request(Operation::Capabilities).await?
                    else {
                        connection.close().await;
                        return Err("capabilities unavailable".into());
                    };
                    self.snapshot.wifi_available =
                        bits & zorvinkel_protocol::wifi::CAP_WIFI_TEMPORARY != 0;
                    self.snapshot.power_available =
                        bits & zorvinkel_protocol::power::CAP_POWER != 0;
                    self.snapshot.tunnel_available =
                        bits & zorvinkel_protocol::tunnel::CAP_TUNNEL != 0;
                    self.snapshot.wifi_persistence = None;
                    self.snapshot.system_info_available =
                        bits & zorvinkel_protocol::system::CAP_SYSTEM_INFO != 0;
                    self.snapshot.system_info = None;
                    self.snapshot.services_available =
                        bits & zorvinkel_protocol::services::CAP_SERVICES != 0;
                    self.snapshot.services.clear();
                    self.snapshot.service_actions_available =
                        bits & zorvinkel_protocol::services::CAP_SERVICE_ACTIONS != 0;
                    self.snapshot.access_point_available =
                        bits & zorvinkel_protocol::ap::CAP_ACCESS_POINT != 0;
                    self.ip_endpoint = bits & zorvinkel_protocol::ip::CAP_IP_REACHABILITY != 0;
                    self.correlation = bits & zorvinkel_protocol::ip::CAP_CORRELATION != 0;
                    self.reachability = None;
                    {
                        use zorvinkel_application::trust::DeviceTrustStore;
                        self.device_key = trust
                            .update(|binding| Ok(binding.expected_key().cloned()))
                            .map_err(|_| "device binding unavailable")?;
                    }
                    if bits & zorvinkel_protocol::wifi::CAP_WIFI_PERSISTENCE != 0 {
                        self.snapshot.wifi_persistence =
                            Some(read_persistence(&mut connection, None).await?);
                    }
                    self.snapshot.restart_policy = None;
                    self.snapshot.restart_differs = false;
                    if bits & zorvinkel_protocol::restart::CAP_RESTART_POLICY != 0 {
                        read_restart_into(&mut connection, &mut self.snapshot).await?;
                    }
                    if let Some(advertised) = self.advertised_names.get(&handle) {
                        use zorvinkel_application::trust::DeviceTrustStore;
                        let remembered = (|| {
                            let key = trust
                                .update(|binding| Ok(binding.expected_key().cloned()))
                                .map_err(|_| ())?
                                .ok_or(())?;
                            let identity = key.as_bytes().try_into().map_err(|_| ())?;
                            let name = zorvinkel_application::device_names::RegisteredName {
                                handle: handle.clone(),
                                identity,
                                hostname: zorvinkel_application::device_names::hostname(advertised)
                                    .into(),
                            };
                            crate::persistence::names::remember(&self.root, name).map_err(|_| ())
                        })();
                        if remembered.is_err() {
                            eprintln!("Device name metadata unavailable");
                        }
                        for device in &mut self.snapshot.devices {
                            if let Some(name) = self.advertised_names.get(&device.handle) {
                                device.display_name = name.clone();
                            }
                        }
                        refresh_names(&self.root, &mut self.snapshot.devices);
                    }
                    if self.snapshot.access_point_available {
                        let stored = read_access_point(&mut connection).await?;
                        self.snapshot.access_point_proposed = stored.is_none();
                        // Without randomness no proposal is offered; the user types one.
                        self.snapshot.access_point = stored.or_else(|| {
                            let hostname = self.advertised_names.get(&handle).map_or("", |name| {
                                zorvinkel_application::device_names::hostname(name)
                            });
                            propose_access_point(hostname, |bytes| getrandom::fill(bytes).is_ok())
                        });
                    }
                    self.connected_handle = Some(handle.clone());
                    self.connection = Some(connection);
                    self.snapshot.status =
                        Some(read_status(self.connection.as_mut().ok_or("session closed")?).await?);
                    if self.snapshot.system_info_available {
                        self.snapshot.system_info =
                            read_system_info(self.connection.as_mut().ok_or("session closed")?)
                                .await?;
                    }
                    if self.snapshot.services_available {
                        self.snapshot.services =
                            read_services(self.connection.as_mut().ok_or("session closed")?)
                                .await?;
                    }
                    observe_reachability(
                        &mut self.snapshot,
                        &mut self.reachability,
                        self.ip_endpoint
                            .then_some(self.device_key.as_ref())
                            .flatten(),
                        &self.identity,
                        self.connection
                            .as_ref()
                            .is_some_and(StatusConnection::is_ip),
                    )
                    .await;
                    if let Some(id) = self.pending.get(&handle).copied() {
                        finish_wifi(
                            self.connection.as_mut().ok_or("session closed")?,
                            &mut self.snapshot,
                            &mut self.pending,
                            &handle,
                            id,
                        )
                        .await?;
                    }
                    prefer_ip(
                        &mut self.connection,
                        self.connected_handle
                            .as_deref()
                            .filter(|_| self.correlation && !self.force_ble),
                        self.device_key.as_ref(),
                        &self.snapshot,
                        &self.pending,
                        &self.root,
                        &self.identity,
                    )
                    .await;
                }
                SetupRequest::Disconnect => {
                    if let Some(mut connection) = self.connection.take() {
                        connection.close().await;
                    }
                    reset_device(&mut self.snapshot);
                    self.connected_handle = None;
                }
                SetupRequest::Refresh => {
                    if let Some(connection) = &mut self.connection {
                        if let Some(handle) = &self.connected_handle
                            && let Some(id) = self.pending.get(handle).copied()
                        {
                            finish_wifi(
                                connection,
                                &mut self.snapshot,
                                &mut self.pending,
                                handle,
                                id,
                            )
                            .await?;
                        }
                        self.snapshot.status = Some(read_status(connection).await?);
                        if self.snapshot.restart_policy.is_some() {
                            read_restart_into(connection, &mut self.snapshot).await?;
                        }
                        if self.snapshot.system_info_available {
                            self.snapshot.system_info = read_system_info(connection).await?;
                        }
                        if self.snapshot.services_available {
                            self.snapshot.services = read_services(connection).await?;
                        }
                        observe_reachability(
                            &mut self.snapshot,
                            &mut self.reachability,
                            self.ip_endpoint
                                .then_some(self.device_key.as_ref())
                                .flatten(),
                            &self.identity,
                            connection.is_ip(),
                        )
                        .await;
                    }
                    prefer_ip(
                        &mut self.connection,
                        self.connected_handle
                            .as_deref()
                            .filter(|_| self.correlation && !self.force_ble),
                        self.device_key.as_ref(),
                        &self.snapshot,
                        &self.pending,
                        &self.root,
                        &self.identity,
                    )
                    .await;
                }
                SetupRequest::SetWifiPersistence(enabled) => {
                    if self.snapshot.wifi_persistence.is_none() {
                        return Err(WifiRequestFailure(SetupError::Unsupported).into());
                    }
                    let connection = self.connection.as_mut().ok_or("session closed")?;
                    self.snapshot.wifi_persistence =
                        Some(read_persistence(connection, Some(enabled)).await?);
                    // Choosing between keeping and the original settings ends a fixed default.
                    if matches!(
                        self.snapshot.restart_policy,
                        Some(zorvinkel_core::network::RestartPolicy::DefaultProfile(_))
                    ) && let Some(handle) = self.connected_handle.clone()
                    {
                        change_restart_policy(
                            connection,
                            &mut self.snapshot,
                            &mut self.pending,
                            &handle,
                            zorvinkel_protocol::restart::RestartChange::LastAccepted,
                        )
                        .await?;
                    }
                }
                SetupRequest::RestartLastAccepted
                | SetupRequest::RestartCurrentAsDefault
                | SetupRequest::RestartRevert => {
                    if self.snapshot.restart_policy.is_none() {
                        return Err(WifiRequestFailure(SetupError::Unsupported).into());
                    }
                    let handle = self.connected_handle.clone().ok_or("session closed")?;
                    let connection = self.connection.as_mut().ok_or("session closed")?;
                    if let Some(id) = self.pending.get(&handle).copied() {
                        finish_wifi(
                            connection,
                            &mut self.snapshot,
                            &mut self.pending,
                            &handle,
                            id,
                        )
                        .await?;
                        return Err(WifiRequestFailure(SetupError::Busy).into());
                    }
                    let change = if matches!(request, SetupRequest::RestartLastAccepted) {
                        zorvinkel_protocol::restart::RestartChange::LastAccepted
                    } else if matches!(request, SetupRequest::RestartRevert) {
                        zorvinkel_protocol::restart::RestartChange::Revert
                    } else {
                        // A default needs something to fix; the device checks this again.
                        if matches!(
                            self.snapshot.status.as_ref().map(|status| &status.network),
                            None | Some(NetworkStatus::Disconnected)
                        ) {
                            return Err(WifiRequestFailure(SetupError::NoActiveNetwork).into());
                        }
                        zorvinkel_protocol::restart::RestartChange::CurrentAsDefault
                    };
                    change_restart_policy(
                        connection,
                        &mut self.snapshot,
                        &mut self.pending,
                        &handle,
                        change,
                    )
                    .await?;
                }
                SetupRequest::ServiceAction { name, action } => {
                    if !self.snapshot.service_actions_available {
                        return Err(WifiRequestFailure(SetupError::Unsupported).into());
                    }
                    let connection = self.connection.as_mut().ok_or("session closed")?;
                    request_service_action(connection, &name, action).await?;
                    // The helper acts asynchronously; give it a moment before reading the state.
                    tokio::time::sleep(SERVICE_ACTION_SETTLE).await;
                    self.snapshot.services = read_services(connection).await?;
                }
                SetupRequest::PowerDevice(action) => {
                    if !self.snapshot.power_available {
                        return Err(WifiRequestFailure(SetupError::Unsupported).into());
                    }
                    let connection = self.connection.as_mut().ok_or("session closed")?;
                    request_power(connection, action).await?;
                    // The device goes away on purpose; keep no stale session or status.
                    if let Some(mut connection) = self.connection.take() {
                        connection.close().await;
                    }
                    reset_device(&mut self.snapshot);
                    self.connected_handle = None;
                }
                SetupRequest::EnableAccessPoint(_)
                | SetupRequest::DisableAccessPoint
                | SetupRequest::UpdateAccessPoint(_) => {
                    if !self.snapshot.access_point_available {
                        return Err(WifiRequestFailure(SetupError::Unsupported).into());
                    }
                    let handle = self.connected_handle.clone().ok_or("session closed")?;
                    let connection = self.connection.as_mut().ok_or("session closed")?;
                    if let Some(id) = self.pending.get(&handle).copied() {
                        finish_wifi(
                            connection,
                            &mut self.snapshot,
                            &mut self.pending,
                            &handle,
                            id,
                        )
                        .await?;
                        return Err(WifiRequestFailure(SetupError::Busy).into());
                    }
                    let active = matches!(
                        self.snapshot.status.as_ref().map(|status| &status.network),
                        Some(NetworkStatus::AccessPointActive { .. })
                    );
                    let settings = match &request {
                        SetupRequest::EnableAccessPoint(settings)
                        | SetupRequest::UpdateAccessPoint(settings) => {
                            validate_access_point(&settings.ssid, &settings.password)
                                .map_err(|_| WifiRequestFailure(SetupError::InvalidInput))?;
                            Some(settings.clone())
                        }
                        _ => None,
                    };
                    // Credentials only change while the access point is off.
                    if active && settings.is_some() {
                        switch_access_point(
                            connection,
                            &mut self.snapshot,
                            &mut self.pending,
                            &handle,
                            false,
                        )
                        .await?;
                    }
                    if let Some(settings) = &settings {
                        write_access_point(connection, settings).await?;
                        self.snapshot.access_point = Some(settings.clone());
                        self.snapshot.access_point_proposed = false;
                    }
                    let enable = match request {
                        SetupRequest::EnableAccessPoint(_) => true,
                        SetupRequest::UpdateAccessPoint(_) => active,
                        _ => false,
                    };
                    if enable || (settings.is_none() && active) {
                        switch_access_point(
                            connection,
                            &mut self.snapshot,
                            &mut self.pending,
                            &handle,
                            enable,
                        )
                        .await?;
                    }
                }
                SetupRequest::Scan | SetupRequest::ConnectWifi { .. } => {
                    if !self.snapshot.wifi_available {
                        return Err(WifiRequestFailure(SetupError::Unsupported).into());
                    }
                    let handle = self.connected_handle.clone().ok_or("session closed")?;
                    if let Some(id) = self.pending.get(&handle).copied() {
                        finish_wifi(
                            self.connection.as_mut().ok_or("session closed")?,
                            &mut self.snapshot,
                            &mut self.pending,
                            &handle,
                            id,
                        )
                        .await?;
                        return Err(WifiRequestFailure(SetupError::Busy).into());
                    }
                    let mut bytes = [0; 8];
                    getrandom::fill(&mut bytes).map_err(|_| "operation identifier unavailable")?;
                    let id = u64::from_le_bytes(bytes).max(1);
                    let (operation, payload) = match request {
                        SetupRequest::ConnectWifi { ssid, password } => {
                            let password = zeroize::Zeroizing::new(password);
                            validate_wifi(&ssid, &password)
                                .map_err(|_| WifiRequestFailure(SetupError::InvalidInput))?;
                            (
                                Operation::WifiConnect,
                                zeroize::Zeroizing::new(zorvinkel_protocol::wifi::connect_payload(
                                    id, &ssid, &password,
                                )?),
                            )
                        }
                        _ => (
                            Operation::WifiScan,
                            zeroize::Zeroizing::new(zorvinkel_protocol::wifi::id_payload(id)?),
                        ),
                    };
                    self.pending.insert(handle.clone(), id);
                    let connection = self.connection.as_mut().ok_or("session closed")?;
                    let response = connection.request_payload(operation, &payload).await?;
                    drop(payload);
                    match response {
                        ResponseBody::WifiScanStarted(value)
                        | ResponseBody::WifiConnectStarted(value)
                            if value == id => {}
                        ResponseBody::Error(error) => {
                            self.pending.remove(&handle);
                            return Err(WifiRequestFailure(wifi_remote_error(error)).into());
                        }
                        _ => return Err("invalid Wi-Fi response".into()),
                    }
                    finish_wifi(
                        connection,
                        &mut self.snapshot,
                        &mut self.pending,
                        &handle,
                        id,
                    )
                    .await?;
                }
                _ => return Err("function unavailable".into()),
            }
            Ok(())
        })
    }
}
/// Moves a Bluetooth session to the network once the device proved it is reachable there, so
/// all further information flows over the network. Any failure keeps the Bluetooth session.
async fn prefer_ip(
    connection: &mut Option<StatusConnection>,
    handle: Option<&str>,
    device_key: Option<&DeviceKey>,
    snapshot: &SetupSnapshot,
    pending: &BTreeMap<String, u64>,
    root: &Path,
    identity: &IdentityHandle,
) {
    // `handle` is only given when the device offers the one-time proof that ties the network
    // session to this one.
    let (Some(handle), Some(device_key), Some(info)) =
        (handle, device_key, snapshot.system_info.as_ref())
    else {
        return;
    };
    let reachable = snapshot
        .status
        .as_ref()
        .is_some_and(|status| status.ip_reachability == IpReachability::Authenticated);
    if !reachable || pending.contains_key(handle) {
        return;
    }
    if connection.as_ref().is_none_or(StatusConnection::is_ip) {
        return;
    }
    let addresses: Vec<IpAddr> = info.addresses.iter().map(|entry| entry.address).collect();
    let Ok(mut trust) = catalog::select(root, handle) else {
        return;
    };
    let Some(current) = connection.as_mut() else {
        return;
    };
    let Ok(ResponseBody::Correlation(token)) = current.request(Operation::CorrelationIssue).await
    else {
        return;
    };
    let token = zeroize::Zeroizing::new(token.0);
    let attempt = timeout(
        Duration::from_secs(10),
        StatusConnection::connect_ip(
            &addresses,
            zorvinkel_protocol::ip::PORT,
            identity,
            &mut trust,
        ),
    )
    .await;
    let Ok(Ok(mut upgraded)) = attempt else {
        return;
    };
    // The handshake checked the pinned key; the new session still has to authenticate itself.
    let proven = {
        use zorvinkel_application::trust::DeviceTrustStore;
        trust
            .update(|binding| Ok(binding.expected_key().cloned()))
            .ok()
            .flatten()
            .is_some_and(|key| &key == device_key)
    };
    // The device only accepts the proof from the same client over the network endpoint.
    if !proven
        || !matches!(
            upgraded.request(Operation::Authenticate).await,
            Ok(ResponseBody::Authenticated)
        )
        || !matches!(
            upgraded
                .request_payload(Operation::CorrelationRedeem, &*token)
                .await,
            Ok(ResponseBody::Correlated)
        )
    {
        upgraded.close().await;
        return;
    }
    if let Some(mut previous) = connection.replace(upgraded) {
        previous.close().await;
    }
}
impl SetupBackend for NativeSetup {
    fn handle(&mut self, request: SetupRequest) -> Result<SetupUpdate, SetupError> {
        if matches!(request, SetupRequest::OpenTunnel) {
            return Ok(self.open_tunnel_update());
        }
        if let SetupRequest::ReadSavedCredentials { ssid } = &request {
            // The system may not know the network; then the password Zorvinkel remembered after
            // a successful connection is the second place to look.
            let result = crate::credentials::SystemCredentialSource
                .saved_wifi_password(ssid)
                .or_else(|_| {
                    crate::wifi_cache::system_wifi_cache()
                        .ok_or(SetupError::CredentialsUnavailable)
                        .and_then(|mut cache| cache.cached_password(ssid))
                        .and_then(|found| found.ok_or(SetupError::CredentialsUnavailable))
                });
            return Ok(SetupUpdate {
                snapshot: self.snapshot.clone(),
                error: result.as_ref().err().copied(),
                saved_password: result.ok(),
            });
        }
        let accepted = match &request {
            SetupRequest::ConnectWifi { ssid, password } => {
                Some((ssid.clone(), zeroize::Zeroizing::new(password.clone())))
            }
            _ => None,
        };
        let was_ip = self
            .connection
            .as_ref()
            .is_some_and(StatusConnection::is_ip);
        let expired = self
            .connection
            .as_ref()
            .and_then(StatusConnection::session_remaining)
            == Some(Duration::ZERO);
        let continues_session =
            !matches!(request, SetupRequest::Connect(_) | SetupRequest::Discover);
        let is_refresh = matches!(request, SetupRequest::Refresh);
        // These requests change the device's own network, so the session over IP is expected to end.
        let changes_network = matches!(
            request,
            SetupRequest::ConnectWifi { .. }
                | SetupRequest::EnableAccessPoint(_)
                | SetupRequest::DisableAccessPoint
        );
        let known_handle = self.connected_handle.clone();
        let result = self.apply(request);
        if let (Ok(()), Some((ssid, password))) = (&result, accepted)
            && let Some(mut cache) = crate::wifi_cache::system_wifi_cache()
            && cache.remember(&ssid, &password).is_err()
        {
            eprintln!("Wi-Fi password could not be cached");
        }
        let mut error = result.err().map(|error| {
            if let Some(error) = error.downcast_ref::<WifiRequestFailure>() {
                let observed = self
                    .connection
                    .as_mut()
                    .and_then(|connection| self.runtime.block_on(read_status(connection)).ok());
                if let Some(status) = observed {
                    self.snapshot.status = Some(status);
                } else {
                    if let Some(mut connection) = self.connection.take() {
                        self.runtime.block_on(connection.close());
                    }
                    reset_device(&mut self.snapshot);
                }
                return error.0;
            }
            eprintln!("Device request failed: {error}");
            reset_device(&mut self.snapshot);
            if let Some(mut connection) = self.connection.take() {
                self.runtime.block_on(connection.close());
            }
            match error.downcast_ref::<crate::security::SecureError>() {
                Some(crate::security::SecureError::Trust) => SetupError::IdentityRejected,
                Some(crate::security::SecureError::Unauthorized) => SetupError::AccessDenied,
                _ if expired => SetupError::SessionExpired,
                _ => SetupError::Unavailable,
            }
        });
        // A lost network session continues over Bluetooth when the device is in reach there.
        // Security failures and ended sessions are not transport problems and never switch.
        // A task that was started before the session ended is completed over Bluetooth by its
        // operation identifier; when that works the lost session is no failure to report.
        if let (Some(failure), Some(handle)) = (error, known_handle)
            && was_ip
            && continues_session
            && !expired
            && self.connection.is_none()
            && !matches!(
                failure,
                SetupError::IdentityRejected | SetupError::AccessDenied
            )
            && {
                // A network change may also finish before the status read that follows it, which
                // then runs into the session the change itself ended; the new status read over
                // Bluetooth shows the outcome.
                let started = self.pending.contains_key(&handle) || changes_network;
                self.fall_back_to_bluetooth(handle) && started
            }
        {
            error = (!is_refresh && !changes_network).then_some(failure);
        }
        self.snapshot.transport = self.connection.as_ref().map(|connection| {
            if connection.is_ip() {
                Transport::Ip
            } else {
                Transport::Ble
            }
        });
        self.snapshot.session_remaining = self
            .connection
            .as_ref()
            .and_then(StatusConnection::session_remaining);
        Ok(SetupUpdate {
            snapshot: self.snapshot.clone(),
            error,
            saved_password: None,
        })
    }
}
impl NativeSetup {
    /// A refused or failed tunnel is reported in the update; unlike other requests it does not
    /// end the connection, because the status session itself still works.
    fn open_tunnel_update(&mut self) -> SetupUpdate {
        use super::secure_client::TunnelRefused;
        use zorvinkel_protocol::status::RemoteError;
        let error = match self.open_tunnel() {
            Ok(stream) => {
                self.snapshot.tunnel.put(stream);
                None
            }
            Err(error) => {
                eprintln!("Tunnel request failed: {error}");
                Some(
                    match error
                        .downcast_ref::<TunnelRefused>()
                        .map(|refused| refused.0)
                    {
                        Some(RemoteError::Busy) => SetupError::Busy,
                        Some(RemoteError::Unsupported | RemoteError::Unavailable) => {
                            SetupError::Unsupported
                        }
                        _ if self.connection.is_none() => SetupError::Disconnected,
                        _ => SetupError::Unavailable,
                    },
                )
            }
        };
        SetupUpdate {
            snapshot: self.snapshot.clone(),
            error,
            saved_password: None,
        }
    }
}
impl NativeSetup {
    /// Reconnects to the same device over Bluetooth only. Failure leaves no half-open session.
    fn fall_back_to_bluetooth(&mut self, handle: String) -> bool {
        self.force_ble = true;
        let result = self.apply(SetupRequest::Connect(handle));
        self.force_ble = false;
        if let Err(error) = result {
            eprintln!("Fallback to Bluetooth failed: {error}");
            if let Some(mut connection) = self.connection.take() {
                self.runtime.block_on(connection.close());
            }
            reset_device(&mut self.snapshot);
            self.connected_handle = None;
            return false;
        }
        true
    }
}
impl Drop for NativeSetup {
    fn drop(&mut self) {
        if let Some(mut connection) = self.connection.take() {
            self.runtime.block_on(connection.close());
        }
    }
}
/// Prefers the network path to a device that was announced there and falls back to Bluetooth.
/// A device that fails to prove its identity is never retried over the other path.
async fn open_connection(
    central: Option<&ProbeCentral>,
    announced: Option<&crate::discovery::IpDevice>,
    identity: &IdentityHandle,
    handle: &str,
    trust: &mut crate::persistence::trust::FileDeviceTrustStore,
) -> BleResult<StatusConnection> {
    let mut failure: Option<Box<dyn std::error::Error + Send + Sync>> = None;
    if let Some(device) = announced {
        match timeout(
            Duration::from_secs(15),
            StatusConnection::connect_ip(&device.addresses, device.port, identity, trust),
        )
        .await
        {
            Ok(Ok(connection)) => return Ok(connection),
            Ok(Err(error)) => {
                if matches!(
                    error.downcast_ref::<crate::security::SecureError>(),
                    Some(crate::security::SecureError::Trust)
                ) {
                    return Err(error);
                }
                failure = Some(error);
            }
            Err(_) => failure = Some("IP connection timed out".into()),
        }
    }
    let Some(central) = central else {
        return Err(failure.unwrap_or_else(|| "search for devices first".into()));
    };
    let peripheral = match central.peripheral(handle).await {
        Ok(peripheral) => peripheral,
        Err(error) => return Err(failure.unwrap_or(error)),
    };
    let attempt = timeout(
        Duration::from_secs(30),
        StatusConnection::connect(peripheral.clone(), identity, trust),
    )
    .await;
    match attempt {
        Ok(Ok(connection)) => Ok(connection),
        result => {
            let _ = timeout(Duration::from_secs(3), peripheral.disconnect()).await;
            match result {
                Ok(Err(error)) => Err(error),
                _ => Err("connection timed out".into()),
            }
        }
    }
}

async fn read_status(connection: &mut StatusConnection) -> BleResult<DeviceStatus> {
    let network = match connection.request(Operation::Status).await? {
        ResponseBody::Status { mode: 0, .. } => NetworkStatus::Disconnected,
        ResponseBody::Status { mode: 1, ssid } => NetworkStatus::WifiConnected { ssid },
        ResponseBody::Status { mode: 2, ssid } => NetworkStatus::AccessPointActive { ssid },
        _ => return Err("network status unavailable".into()),
    };
    Ok(DeviceStatus {
        network,
        ip_reachability: IpReachability::Unverified,
    })
}

/// Tests whether this computer reaches the device over IP and proves its identity there.
/// A device without the endpoint or without addresses stays unverified; failures never block setup.
async fn observe_reachability(
    snapshot: &mut SetupSnapshot,
    cache: &mut Option<CachedReachability>,
    device_key: Option<&DeviceKey>,
    identity: &IdentityHandle,
    over_ip: bool,
) {
    // A session that runs over IP has already proven reachability and identity.
    if over_ip {
        if let Some(status) = snapshot.status.as_mut() {
            status.ip_reachability = IpReachability::Authenticated;
        }
        return;
    }
    let (Some(device_key), Some(status), Some(info)) =
        (device_key, snapshot.status.as_mut(), &snapshot.system_info)
    else {
        return;
    };
    let addresses = info
        .addresses
        .iter()
        .map(|entry| entry.address)
        .collect::<Vec<_>>();
    status.ip_reachability = match cache {
        Some(cached)
            if cached.addresses == addresses && cached.checked.elapsed() < REACHABILITY_TTL =>
        {
            cached.result
        }
        _ => {
            let result = crate::ip::reachability(
                &addresses,
                zorvinkel_protocol::ip::PORT,
                identity,
                device_key,
            )
            .await;
            *cache = Some(CachedReachability {
                addresses,
                checked: Instant::now(),
                result,
            });
            result
        }
    };
}

fn reset_device(snapshot: &mut SetupSnapshot) {
    snapshot.status = None;
    snapshot.wifi_available = false;
    snapshot.power_available = false;
    snapshot.tunnel_available = false;
    snapshot.wifi_persistence = None;
    snapshot.restart_policy = None;
    snapshot.restart_differs = false;
    snapshot.system_info_available = false;
    snapshot.system_info = None;
    snapshot.services_available = false;
    snapshot.services.clear();
    snapshot.service_actions_available = false;
    snapshot.access_point_available = false;
    snapshot.access_point = None;
    snapshot.access_point_proposed = false;
    snapshot.networks.clear();
    snapshot.transport = None;
    snapshot.session_remaining = None;
}

/// A device that cannot read its values right now yields `None`; transport errors propagate.
async fn read_system_info(
    connection: &mut StatusConnection,
) -> BleResult<Option<zorvinkel_application::system::SystemInfo>> {
    match connection.request(Operation::SystemInfo).await? {
        ResponseBody::SystemInfo(info) => Ok(Some(crate::system_info::from_wire(info))),
        ResponseBody::Error(_) => Ok(None),
        _ => Err("invalid system information response".into()),
    }
}

/// A device that cannot read its services right now lists none; transport errors propagate.
async fn read_services(
    connection: &mut StatusConnection,
) -> BleResult<Vec<zorvinkel_application::services::ServiceEntry>> {
    match connection.request(Operation::Services).await? {
        ResponseBody::Services(services) => Ok(crate::services::from_wire(services)),
        ResponseBody::Error(_) => Ok(Vec::new()),
        _ => Err("invalid services response".into()),
    }
}

async fn read_access_point(
    connection: &mut StatusConnection,
) -> BleResult<Option<AccessPointSettings>> {
    match connection.request(Operation::ApRead).await? {
        ResponseBody::ApCredentials(stored) => Ok(stored.map(|stored| AccessPointSettings {
            ssid: stored.ssid,
            password: stored.password,
        })),
        ResponseBody::Error(error) => Err(WifiRequestFailure(wifi_remote_error(error)).into()),
        _ => Err("invalid access point response".into()),
    }
}

async fn write_access_point(
    connection: &mut StatusConnection,
    settings: &AccessPointSettings,
) -> BleResult<()> {
    let payload = zeroize::Zeroizing::new(zorvinkel_protocol::ap::credentials_payload(
        &settings.ssid,
        &settings.password,
    )?);
    match connection
        .request_payload(Operation::ApWrite, &payload)
        .await?
    {
        ResponseBody::ApStored => Ok(()),
        ResponseBody::Error(error) => Err(WifiRequestFailure(wifi_remote_error(error)).into()),
        _ => Err("invalid access point response".into()),
    }
}

/// Starts the switch on the device and follows it like any other network task.
async fn switch_access_point(
    connection: &mut StatusConnection,
    snapshot: &mut SetupSnapshot,
    pending: &mut BTreeMap<String, u64>,
    handle: &str,
    enable: bool,
) -> BleResult<()> {
    let mut bytes = [0; 8];
    getrandom::fill(&mut bytes).map_err(|_| "operation identifier unavailable")?;
    let id = u64::from_le_bytes(bytes).max(1);
    let payload = zorvinkel_protocol::ap::id_payload(id)?;
    pending.insert(handle.into(), id);
    let operation = if enable {
        Operation::ApEnable
    } else {
        Operation::ApDisable
    };
    match connection.request_payload(operation, &payload).await? {
        ResponseBody::ApStarted(value) if value == id => {}
        ResponseBody::Error(error) => {
            pending.remove(handle);
            return Err(WifiRequestFailure(wifi_remote_error(error)).into());
        }
        _ => return Err("invalid access point response".into()),
    }
    finish_wifi(connection, snapshot, pending, handle, id).await
}

/// The client only needs to know whether a default is set; the stored profile stays on the
/// device, so the reference below is a placeholder for the discriminant.
async fn read_restart_into(
    connection: &mut StatusConnection,
    snapshot: &mut SetupSnapshot,
) -> BleResult<()> {
    let (policy, differs) = read_restart_policy(connection).await?;
    snapshot.restart_policy = Some(policy);
    snapshot.restart_differs = differs;
    Ok(())
}
async fn read_restart_policy(
    connection: &mut StatusConnection,
) -> BleResult<(zorvinkel_core::network::RestartPolicy, bool)> {
    use zorvinkel_core::network::{NetworkConfiguration, ProfileId, RestartPolicy};
    use zorvinkel_protocol::restart::RestartState;
    match connection.request(Operation::RestartPolicyRead).await? {
        ResponseBody::RestartPolicy(report) => {
            let reference = || ProfileId::new("device-default".into());
            let policy = match report.state {
                RestartState::LastAccepted => RestartPolicy::LastAccepted,
                RestartState::DefaultWifi => RestartPolicy::DefaultProfile(
                    NetworkConfiguration::Wifi(reference().map_err(|_| "invalid profile")?),
                ),
                RestartState::DefaultAccessPoint => RestartPolicy::DefaultProfile(
                    NetworkConfiguration::AccessPoint(reference().map_err(|_| "invalid profile")?),
                ),
            };
            Ok((policy, report.differs))
        }
        ResponseBody::Error(error) => Err(WifiRequestFailure(wifi_remote_error(error)).into()),
        _ => Err("invalid restart option response".into()),
    }
}

/// Starts the change on the device, follows it like any other network task and reads the
/// option back, so the display shows what the device holds.
async fn change_restart_policy(
    connection: &mut StatusConnection,
    snapshot: &mut SetupSnapshot,
    pending: &mut BTreeMap<String, u64>,
    handle: &str,
    change: zorvinkel_protocol::restart::RestartChange,
) -> BleResult<()> {
    let mut bytes = [0; 8];
    getrandom::fill(&mut bytes).map_err(|_| "operation identifier unavailable")?;
    let id = u64::from_le_bytes(bytes).max(1);
    let payload = zorvinkel_protocol::restart::set_payload(id, change)?;
    pending.insert(handle.into(), id);
    match connection
        .request_payload(Operation::RestartPolicySet, &payload)
        .await?
    {
        ResponseBody::RestartPolicyStarted(value) if value == id => {}
        ResponseBody::Error(error) => {
            pending.remove(handle);
            return Err(WifiRequestFailure(wifi_remote_error(error)).into());
        }
        _ => return Err("invalid restart option response".into()),
    }
    // The device refuses a default while it has no network; the Wi-Fi task state for a refusal
    // is the same as for a rejected connection, so it is named for what it means here.
    finish_wifi(connection, snapshot, pending, handle, id)
        .await
        .map_err(|error| match error.downcast_ref::<WifiRequestFailure>() {
            Some(WifiRequestFailure(SetupError::ConnectionRejected)) => {
                WifiRequestFailure(SetupError::NoActiveNetwork).into()
            }
            _ => error,
        })?;
    read_restart_into(connection, snapshot).await
}

async fn read_persistence(
    connection: &mut StatusConnection,
    change: Option<bool>,
) -> BleResult<bool> {
    let payload = zorvinkel_protocol::wifi::persistence_payload(change);
    match connection
        .request_payload(Operation::WifiPersistence, &payload)
        .await?
    {
        ResponseBody::WifiPersistence(enabled) if change.is_none_or(|wanted| wanted == enabled) => {
            Ok(enabled)
        }
        ResponseBody::Error(error) => Err(WifiRequestFailure(wifi_remote_error(error)).into()),
        _ => Err("invalid Wi-Fi persistence response".into()),
    }
}

async fn request_service_action(
    connection: &mut StatusConnection,
    name: &str,
    action: zorvinkel_application::services::ServiceAction,
) -> BleResult<()> {
    let wire = crate::services::action_to_wire(action);
    let payload = zorvinkel_protocol::services::action_payload(wire, name)?;
    match connection
        .request_payload(Operation::ServiceAction, &payload)
        .await?
    {
        ResponseBody::ServiceActionAccepted(accepted, accepted_name)
            if accepted == wire && accepted_name == name =>
        {
            Ok(())
        }
        ResponseBody::Error(error) => Err(WifiRequestFailure(wifi_remote_error(error)).into()),
        _ => Err("invalid service action response".into()),
    }
}

async fn request_power(
    connection: &mut StatusConnection,
    action: zorvinkel_application::power::PowerAction,
) -> BleResult<()> {
    use zorvinkel_application::power::PowerAction;
    use zorvinkel_protocol::power::{PowerCommand, power_payload};
    let command = match action {
        PowerAction::Shutdown => PowerCommand::Shutdown,
        PowerAction::Restart => PowerCommand::Restart,
    };
    match connection
        .request_payload(Operation::Power, &power_payload(command))
        .await?
    {
        ResponseBody::PowerAccepted(accepted) if accepted == command => Ok(()),
        ResponseBody::Error(error) => Err(WifiRequestFailure(wifi_remote_error(error)).into()),
        _ => Err("invalid power response".into()),
    }
}

#[derive(Debug)]
struct WifiRequestFailure(SetupError);
impl std::fmt::Display for WifiRequestFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Wi-Fi operation failed")
    }
}
impl std::error::Error for WifiRequestFailure {}
fn wifi_remote_error(error: zorvinkel_protocol::status::RemoteError) -> SetupError {
    use zorvinkel_protocol::status::RemoteError;
    match error {
        RemoteError::Busy => SetupError::Busy,
        RemoteError::Conflict => SetupError::Unavailable,
        RemoteError::Unsupported => SetupError::Unsupported,
        RemoteError::Unauthorized | RemoteError::RegistrationClosed => SetupError::AccessDenied,
        _ => SetupError::Unavailable,
    }
}
async fn finish_wifi(
    connection: &mut StatusConnection,
    snapshot: &mut SetupSnapshot,
    pending: &mut BTreeMap<String, u64>,
    handle: &str,
    id: u64,
) -> BleResult<()> {
    let started = tokio::time::Instant::now();
    let payload = zorvinkel_protocol::wifi::id_payload(id)?;
    loop {
        match connection
            .request_payload(Operation::WifiPoll, &payload)
            .await?
        {
            ResponseBody::WifiTask(task) if task.id == id => {
                if task.state == 0 {
                    if started.elapsed() > Duration::from_secs(120) {
                        return Err(WifiRequestFailure(SetupError::Busy).into());
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    continue;
                }
                pending.remove(handle);
                match task.state {
                    1 => snapshot.networks = task.networks,
                    2 => (),
                    3 => return Err(WifiRequestFailure(SetupError::ConnectionRejected).into()),
                    5 => return Err(WifiRequestFailure(SetupError::RecoveryFailed).into()),
                    _ => return Err(WifiRequestFailure(SetupError::Unavailable).into()),
                }
                snapshot.status = Some(read_status(connection).await?);
                if snapshot.restart_policy.is_some() {
                    read_restart_into(connection, snapshot).await?;
                }
                return Ok(());
            }
            ResponseBody::Error(error) => {
                pending.remove(handle);
                return Err(WifiRequestFailure(wifi_remote_error(error)).into());
            }
            _ => return Err("invalid Wi-Fi task response".into()),
        }
    }
}

fn refresh_names(root: &Path, devices: &mut [zorvinkel_core::DiscoveredDevice]) {
    match crate::persistence::names::read(root) {
        Ok(registered) => zorvinkel_application::device_names::apply(devices, &registered),
        Err(_) => {
            eprintln!("Device name metadata unavailable");
            zorvinkel_application::device_names::apply(devices, &[]);
        }
    }
}
