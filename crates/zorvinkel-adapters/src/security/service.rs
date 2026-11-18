use super::{SecureError, channel::SecureChannel, handshake::Role};
use std::{
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use zeroize::Zeroizing;
use zorvinkel_application::{
    device::NetworkStatusSource,
    registration::register,
    state::{ApplicationError, StateStore},
    status::{authenticate, read_network_status},
};
use zorvinkel_core::{
    NetworkStatus,
    registry::{ClientKey, RegistryError},
};
use zorvinkel_protocol::{ip::CorrelationToken, status::*};

#[derive(Clone, Copy)]
pub struct ServiceTime {
    pub elapsed: Duration,
    pub registration_seconds: u64,
}

/// IP sessions may use at most this many of the session slots, so a network peer cannot
/// keep Bluetooth setup from connecting.
const MAX_IP_SESSIONS: usize = 2;
const MAX_SESSIONS: usize = 4;

/// How long an issued correlation proof can be redeemed.
const CORRELATION_VALIDITY: Duration = Duration::from_secs(30);
/// Open proofs at most; issuing more drops the oldest, so the list stays small.
const MAX_PROOFS: usize = 8;

struct Proof {
    token: [u8; zorvinkel_protocol::ip::CORRELATION_LEN],
    client: Vec<u8>,
    issued: Duration,
}
/// One-time proofs that tie an IP session to a running Bluetooth session of the same client.
#[derive(Clone, Default)]
struct CorrelationBook(Arc<Mutex<Vec<Proof>>>);
impl CorrelationBook {
    fn issue(&self, client: &ClientKey, now: Duration) -> Option<CorrelationToken> {
        let mut token = [0; zorvinkel_protocol::ip::CORRELATION_LEN];
        getrandom::fill(&mut token).ok()?;
        let mut proofs = self.0.lock().ok()?;
        proofs.retain(|proof| now.saturating_sub(proof.issued) < CORRELATION_VALIDITY);
        if proofs.len() >= MAX_PROOFS {
            proofs.remove(0);
        }
        proofs.push(Proof {
            token,
            client: client.as_bytes().to_vec(),
            issued: now,
        });
        Some(CorrelationToken(token))
    }
    /// A proof works once and only for the client it was issued to.
    fn redeem(&self, client: &ClientKey, token: &CorrelationToken, now: Duration) -> bool {
        let Ok(mut proofs) = self.0.lock() else {
            return false;
        };
        proofs.retain(|proof| now.saturating_sub(proof.issued) < CORRELATION_VALIDITY);
        let found = proofs.iter().position(|proof| {
            proof.client == client.as_bytes() && constant_time_equal(&proof.token, &token.0)
        });
        found.map(|index| proofs.remove(index)).is_some()
    }
}
fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

struct DeviceSessionState {
    channel: SecureChannel,
    correlations: CorrelationBook,
    authorized: bool,
    next_id: u64,
    /// The session arrived over IP; it counts against the IP session slots.
    over_ip: bool,
}

pub struct DeviceSession(Arc<Mutex<DeviceSessionState>>);
impl DeviceSession {
    pub fn new(channel: SecureChannel, sessions: &SessionRegistry) -> Result<Self, SecureError> {
        Self::attach(channel, sessions, false)
    }
    /// A session of the IP endpoint; it offers the same operations as a Bluetooth session.
    pub fn new_ip(channel: SecureChannel, sessions: &SessionRegistry) -> Result<Self, SecureError> {
        Self::attach(channel, sessions, true)
    }
    fn attach(
        channel: SecureChannel,
        sessions: &SessionRegistry,
        over_ip: bool,
    ) -> Result<Self, SecureError> {
        if channel.role != Role::Device {
            return Err(SecureError::InvalidMessage);
        }
        let state = Arc::new(Mutex::new(DeviceSessionState {
            channel,
            authorized: false,
            next_id: 1,
            over_ip,
            correlations: sessions.correlations.clone(),
        }));
        sessions.attach(&state)?;
        Ok(Self(state))
    }
    pub fn is_closed(&self) -> bool {
        self.0
            .lock()
            .map_or(true, |state| state.channel.is_closed())
    }
    pub fn close(&mut self) {
        if let Ok(mut state) = self.0.lock() {
            state.channel.close();
            state.authorized = false;
        }
    }
    pub fn handle(
        &mut self,
        record: &[u8],
        store: &mut impl StateStore,
        network: &mut impl NetworkStatusSource,
        time: ServiceTime,
    ) -> Result<Vec<u8>, SecureError> {
        let mut state = self.0.lock().map_err(|_| SecureError::Closed)?;
        let result = state.handle_inner(record, store, network, time);
        if result.is_err() {
            state.channel.close();
            state.authorized = false;
        }
        result
    }
}
impl DeviceSessionState {
    fn handle_inner(
        &mut self,
        record: &[u8],
        store: &mut impl StateStore,
        network: &mut impl NetworkStatusSource,
        time: ServiceTime,
    ) -> Result<Vec<u8>, SecureError> {
        if !self.authorized
            && time.elapsed.saturating_sub(self.channel.started()) >= Duration::from_secs(10)
        {
            return Err(SecureError::TimedOut);
        }
        let plaintext = self.channel.decrypt(record, time.elapsed)?;
        let request = decode_request(&plaintext)?;
        if request.id != self.next_id {
            return Err(SecureError::InvalidMessage);
        }
        self.next_id = self.next_id.checked_add(1).ok_or(SecureError::Limit)?;
        let actor = ClientKey::new(self.channel.peer_key().to_vec())
            .map_err(|_| SecureError::InvalidIdentity)?;
        let body = match self.dispatch(
            request.operation,
            request_payload(&plaintext)?,
            &actor,
            store,
            network,
            time,
        ) {
            Ok(body) => body,
            Err(error) => ResponseBody::Error(remote_error(error)),
        };
        let close = matches!(
            body,
            ResponseBody::Error(RemoteError::Unauthorized | RemoteError::RegistrationClosed)
        );
        let response = Zeroizing::new(encode_response(&Response {
            id: request.id,
            body,
        })?);
        let record = self.channel.encrypt(&response, time.elapsed)?;
        if close {
            self.channel.close();
            self.authorized = false;
        }
        Ok(record)
    }
    fn dispatch(
        &mut self,
        operation: Operation,
        payload: &[u8],
        actor: &ClientKey,
        store: &mut impl StateStore,
        network: &mut impl NetworkStatusSource,
        time: ServiceTime,
    ) -> Result<ResponseBody, ApplicationError> {
        let ServiceTime {
            elapsed: now,
            registration_seconds,
        } = time;
        match operation {
            Operation::Register => {
                register(store, actor.clone(), registration_seconds)?;
                self.authorized = true;
                return Ok(ResponseBody::Registered);
            }
            Operation::Authenticate => {
                authenticate(store, actor)?;
                self.authorized = true;
                return Ok(ResponseBody::Authenticated);
            }
            _ => (),
        }
        if !self.authorized {
            return Err(RegistryError::Unauthorized.into());
        }
        authenticate(store, actor)?;
        match operation {
            Operation::Status => {
                if let Some(wifi) = network.wifi_setup() {
                    wifi.client_active(actor);
                }
                match read_network_status(store, network, actor)? {
                    NetworkStatus::Disconnected => Ok(ResponseBody::Status {
                        mode: 0,
                        ssid: String::new(),
                    }),
                    NetworkStatus::WifiConnected { ssid } => {
                        Ok(ResponseBody::Status { mode: 1, ssid })
                    }
                    NetworkStatus::AccessPointActive { ssid } => {
                        Ok(ResponseBody::Status { mode: 2, ssid })
                    }
                }
            }
            Operation::CorrelationIssue => Ok(match self.correlations.issue(actor, now) {
                Some(token) => ResponseBody::Correlation(token),
                None => ResponseBody::Error(RemoteError::Unavailable),
            }),
            Operation::CorrelationRedeem => {
                let token = zorvinkel_protocol::ip::read_correlation(payload)
                    .map_err(|_| RegistryError::Unauthorized)?;
                // Only a session of the IP endpoint continues a Bluetooth session.
                Ok(
                    if self.over_ip && self.correlations.redeem(actor, &token, now) {
                        ResponseBody::Correlated
                    } else {
                        ResponseBody::Error(RemoteError::Conflict)
                    },
                )
            }
            Operation::Capabilities => Ok(ResponseBody::Capabilities(
                CAPABILITIES
                    | if network.ip_endpoint_available() {
                        zorvinkel_protocol::ip::CAP_IP_REACHABILITY
                            | zorvinkel_protocol::ip::CAP_CORRELATION
                    } else {
                        0
                    }
                    | if network.power_control().is_some() {
                        zorvinkel_protocol::power::CAP_POWER
                    } else {
                        0
                    }
                    | if network.system_info().is_some() {
                        zorvinkel_protocol::system::CAP_SYSTEM_INFO
                    } else {
                        0
                    }
                    | if network.service_info().is_some() {
                        zorvinkel_protocol::services::CAP_SERVICES
                    } else {
                        0
                    }
                    | if network.service_control().is_some() {
                        zorvinkel_protocol::services::CAP_SERVICE_ACTIONS
                    } else {
                        0
                    }
                    | match network.wifi_setup() {
                        Some(wifi) if wifi.access_point_supported() => {
                            zorvinkel_protocol::ap::CAP_ACCESS_POINT
                        }
                        _ => 0,
                    }
                    | match network.wifi_setup() {
                        Some(wifi) if wifi.restart_supported() => {
                            zorvinkel_protocol::restart::CAP_RESTART_POLICY
                        }
                        _ => 0,
                    }
                    | match network.wifi_setup() {
                        Some(wifi) if wifi.persistence_supported() => {
                            zorvinkel_protocol::wifi::CAP_WIFI_TEMPORARY
                                | zorvinkel_protocol::wifi::CAP_WIFI_PERSISTENCE
                        }
                        Some(_) => zorvinkel_protocol::wifi::CAP_WIFI_TEMPORARY,
                        None => 0,
                    },
            )),
            Operation::WifiScan
            | Operation::WifiConnect
            | Operation::WifiPoll
            | Operation::WifiPersistence => {
                let Some(wifi) = network.wifi_setup() else {
                    return Ok(ResponseBody::Error(RemoteError::Unsupported));
                };
                Ok(dispatch_wifi(wifi, actor, operation, payload))
            }
            Operation::ApRead | Operation::ApWrite | Operation::ApEnable | Operation::ApDisable => {
                let Some(wifi) = network.wifi_setup() else {
                    return Ok(ResponseBody::Error(RemoteError::Unsupported));
                };
                Ok(dispatch_access_point(wifi, actor, operation, payload))
            }
            Operation::RestartPolicyRead | Operation::RestartPolicySet => {
                let Some(wifi) = network.wifi_setup() else {
                    return Ok(ResponseBody::Error(RemoteError::Unsupported));
                };
                Ok(dispatch_restart(wifi, actor, operation, payload))
            }
            Operation::SystemInfo => Ok(match network.system_info() {
                Some(source) => dispatch_system_info(source),
                None => ResponseBody::Error(RemoteError::Unsupported),
            }),
            Operation::Services => Ok(match network.service_info() {
                Some(source) => dispatch_services(source),
                None => ResponseBody::Error(RemoteError::Unsupported),
            }),
            Operation::ServiceAction => Ok(match network.service_control() {
                Some(control) => dispatch_service_action(control, payload),
                None => ResponseBody::Error(RemoteError::Unsupported),
            }),
            Operation::Power => Ok(match network.power_control() {
                Some(power) => dispatch_power(power, payload),
                None => ResponseBody::Error(RemoteError::Unsupported),
            }),
            _ => Ok(ResponseBody::Error(RemoteError::Unsupported)),
        }
    }
}
fn dispatch_system_info(
    source: &mut dyn zorvinkel_application::system::SystemInfoSource,
) -> ResponseBody {
    match source.read() {
        Ok(info) => ResponseBody::SystemInfo(crate::system_info::to_wire(info)),
        Err(_) => ResponseBody::Error(RemoteError::Unavailable),
    }
}
fn dispatch_services(
    source: &mut dyn zorvinkel_application::services::ServiceInfoSource,
) -> ResponseBody {
    match source.read() {
        Ok(services) => ResponseBody::Services(crate::services::to_wire(services)),
        Err(_) => ResponseBody::Error(RemoteError::Unavailable),
    }
}
fn dispatch_service_action(
    control: &mut dyn zorvinkel_application::services::ServiceControl,
    payload: &[u8],
) -> ResponseBody {
    use zorvinkel_application::services::ServiceControlError;
    let Ok((action, name)) = zorvinkel_protocol::services::read_action(payload) else {
        return ResponseBody::Error(RemoteError::Conflict);
    };
    match control.perform(&name, crate::services::action_from_wire(action)) {
        Ok(()) => ResponseBody::ServiceActionAccepted(action, name),
        Err(ServiceControlError::Unknown | ServiceControlError::NotAllowed) => {
            ResponseBody::Error(RemoteError::Conflict)
        }
        Err(ServiceControlError::Busy) => ResponseBody::Error(RemoteError::Busy),
        Err(ServiceControlError::Unavailable) => ResponseBody::Error(RemoteError::Unavailable),
    }
}
fn dispatch_power(
    power: &mut dyn zorvinkel_application::power::PowerControl,
    payload: &[u8],
) -> ResponseBody {
    use zorvinkel_application::power::{PowerAction, PowerError};
    use zorvinkel_protocol::power::{PowerCommand, read_power};
    let Ok(command) = read_power(payload) else {
        return ResponseBody::Error(RemoteError::Conflict);
    };
    let action = match command {
        PowerCommand::Shutdown => PowerAction::Shutdown,
        PowerCommand::Restart => PowerAction::Restart,
    };
    match power.schedule(action) {
        Ok(()) => ResponseBody::PowerAccepted(command),
        Err(PowerError::Busy) => ResponseBody::Error(RemoteError::Busy),
        Err(PowerError::Unavailable) => ResponseBody::Error(RemoteError::Unavailable),
    }
}
fn dispatch_access_point(
    wifi: &mut dyn zorvinkel_application::wifi::WifiSetupControl,
    actor: &ClientKey,
    operation: Operation,
    payload: &[u8],
) -> ResponseBody {
    use zorvinkel_application::wifi::WifiError;
    use zorvinkel_protocol::ap::{read_credentials, read_id};
    if !wifi.access_point_supported() {
        return ResponseBody::Error(RemoteError::Unsupported);
    }
    let result = (|| {
        Ok(match operation {
            Operation::ApRead => {
                ResponseBody::ApCredentials(wifi.access_point(actor)?.map(|stored| {
                    zorvinkel_protocol::ap::ApCredentials {
                        ssid: stored.ssid,
                        password: stored.password,
                    }
                }))
            }
            Operation::ApWrite => {
                let input = read_credentials(payload).map_err(|_| WifiError::Conflict)?;
                wifi.set_access_point(actor, &input.ssid, &input.password)?;
                ResponseBody::ApStored
            }
            Operation::ApEnable | Operation::ApDisable => {
                let id = read_id(payload).map_err(|_| WifiError::Conflict)?;
                wifi.switch_access_point(actor, id, operation == Operation::ApEnable)?;
                ResponseBody::ApStarted(id)
            }
            _ => return Err(WifiError::Unsupported),
        })
    })();
    result.unwrap_or_else(|error| ResponseBody::Error(wifi_remote_error(error)))
}
fn dispatch_restart(
    wifi: &mut dyn zorvinkel_application::wifi::WifiSetupControl,
    actor: &ClientKey,
    operation: Operation,
    payload: &[u8],
) -> ResponseBody {
    use zorvinkel_application::wifi::{RestartChange, RestartState, WifiError};
    use zorvinkel_protocol::restart as wire;
    if !wifi.restart_supported() {
        return ResponseBody::Error(RemoteError::Unsupported);
    }
    let result = (|| {
        Ok(match operation {
            Operation::RestartPolicyRead => {
                ResponseBody::RestartPolicy(match wifi.restart_state(actor)? {
                    RestartState::LastAccepted => wire::RestartState::LastAccepted,
                    RestartState::DefaultWifi => wire::RestartState::DefaultWifi,
                    RestartState::DefaultAccessPoint => wire::RestartState::DefaultAccessPoint,
                })
            }
            Operation::RestartPolicySet => {
                let request = wire::read_set(payload).map_err(|_| WifiError::Conflict)?;
                wifi.change_restart(
                    actor,
                    request.id,
                    match request.change {
                        wire::RestartChange::LastAccepted => RestartChange::LastAccepted,
                        wire::RestartChange::CurrentAsDefault => RestartChange::CurrentAsDefault,
                    },
                )?;
                ResponseBody::RestartPolicyStarted(request.id)
            }
            _ => return Err(WifiError::Unsupported),
        })
    })();
    result.unwrap_or_else(|error| ResponseBody::Error(wifi_remote_error(error)))
}
fn wifi_remote_error(error: zorvinkel_application::wifi::WifiError) -> RemoteError {
    use zorvinkel_application::wifi::WifiError;
    match error {
        WifiError::Busy => RemoteError::Busy,
        WifiError::Conflict => RemoteError::Conflict,
        WifiError::Unauthorized => RemoteError::Unauthorized,
        WifiError::Unsupported => RemoteError::Unsupported,
        WifiError::Unavailable => RemoteError::Unavailable,
    }
}
fn dispatch_wifi(
    wifi: &mut dyn zorvinkel_application::wifi::WifiSetupControl,
    actor: &ClientKey,
    operation: Operation,
    payload: &[u8],
) -> ResponseBody {
    use zorvinkel_application::wifi::{WifiError, WifiTaskState};
    use zorvinkel_protocol::wifi::{WifiTask, read_connect, read_id, read_persistence};
    let result = (|| {
        Ok(match operation {
            Operation::WifiScan => {
                let id = read_id(payload).map_err(|_| WifiError::Conflict)?;
                wifi.scan(actor, id)?;
                ResponseBody::WifiScanStarted(id)
            }
            Operation::WifiConnect => {
                let input = read_connect(payload).map_err(|_| WifiError::Conflict)?;
                wifi.connect(actor, input.id, input.ssid, input.password)?;
                ResponseBody::WifiConnectStarted(input.id)
            }
            Operation::WifiPoll => {
                let id = read_id(payload).map_err(|_| WifiError::Conflict)?;
                let (state, networks) = match wifi.poll(actor, id)? {
                    WifiTaskState::Pending => (0, vec![]),
                    WifiTaskState::Networks(networks) => (1, networks),
                    WifiTaskState::Connected => (2, vec![]),
                    WifiTaskState::Rejected => (3, vec![]),
                    WifiTaskState::Unavailable => (4, vec![]),
                    WifiTaskState::RecoveryFailed => (5, vec![]),
                };
                ResponseBody::WifiTask(WifiTask {
                    id,
                    state,
                    networks,
                })
            }
            Operation::WifiPersistence => {
                let change = read_persistence(payload).map_err(|_| WifiError::Conflict)?;
                ResponseBody::WifiPersistence(wifi.persistence(actor, change)?)
            }
            _ => return Err(WifiError::Unsupported),
        })
    })();
    result.unwrap_or_else(|error| ResponseBody::Error(wifi_remote_error(error)))
}
fn remote_error(error: ApplicationError) -> RemoteError {
    match error {
        ApplicationError::Registry(RegistryError::Closed) => RemoteError::RegistrationClosed,
        ApplicationError::Registry(RegistryError::Revoked | RegistryError::Unauthorized) => {
            RemoteError::Unauthorized
        }
        _ => RemoteError::Unavailable,
    }
}

pub struct ClientSession {
    channel: SecureChannel,
    next_id: u64,
    pending: Option<Request>,
}
impl ClientSession {
    pub fn new(channel: SecureChannel) -> Result<Self, SecureError> {
        if channel.role != Role::Client {
            return Err(SecureError::InvalidMessage);
        }
        Ok(Self {
            channel,
            next_id: 1,
            pending: None,
        })
    }
    pub fn close(&mut self) {
        self.channel.close();
        self.pending = None;
    }
    /// Time and records the session has left before the secure profile ends it.
    pub fn remaining(&self, now: Duration) -> (Duration, usize) {
        (self.channel.lifetime_left(now), self.channel.records_left())
    }
    pub fn request(&mut self, operation: Operation, now: Duration) -> Result<Vec<u8>, SecureError> {
        self.request_payload(operation, &[], now)
    }
    pub fn request_payload(
        &mut self,
        operation: Operation,
        payload: &[u8],
        now: Duration,
    ) -> Result<Vec<u8>, SecureError> {
        if self.pending.is_some() {
            return Err(SecureError::Busy);
        }
        let request = Request {
            id: self.next_id,
            operation,
        };
        let plaintext = Zeroizing::new(encode_request_payload(request, payload)?);
        let record = self.channel.encrypt(&plaintext, now)?;
        self.next_id = self.next_id.checked_add(1).ok_or(SecureError::Limit)?;
        self.pending = Some(request);
        Ok(record)
    }
    pub fn receive(&mut self, record: &[u8], now: Duration) -> Result<Response, SecureError> {
        let result = (|| {
            let request = self.pending.ok_or(SecureError::InvalidMessage)?;
            let plaintext = self.channel.decrypt(record, now)?;
            let response = decode_response(&plaintext)?;
            if response.id != request.id || !response_matches(&response.body, request.operation) {
                return Err(SecureError::InvalidMessage);
            }
            self.pending = None;
            if matches!(
                response.body,
                ResponseBody::Error(RemoteError::Unauthorized | RemoteError::RegistrationClosed)
            ) {
                self.close();
            }
            Ok(response)
        })();
        if result.is_err() {
            self.close();
        }
        result
    }
}
fn response_matches(response: &ResponseBody, operation: Operation) -> bool {
    matches!(
        (response, operation),
        (ResponseBody::Error(_), _)
            | (ResponseBody::Registered, Operation::Register)
            | (ResponseBody::Authenticated, Operation::Authenticate)
            | (ResponseBody::Status { .. }, Operation::Status)
            | (ResponseBody::Capabilities(_), Operation::Capabilities)
            | (ResponseBody::Correlation(_), Operation::CorrelationIssue)
            | (ResponseBody::Correlated, Operation::CorrelationRedeem)
            | (ResponseBody::WifiScanStarted(_), Operation::WifiScan)
            | (ResponseBody::WifiConnectStarted(_), Operation::WifiConnect)
            | (ResponseBody::WifiTask(_), Operation::WifiPoll)
            | (ResponseBody::WifiPersistence(_), Operation::WifiPersistence)
            | (ResponseBody::RestartPolicy(_), Operation::RestartPolicyRead)
            | (
                ResponseBody::RestartPolicyStarted(_),
                Operation::RestartPolicySet
            )
            | (ResponseBody::PowerAccepted(_), Operation::Power)
            | (ResponseBody::SystemInfo(_), Operation::SystemInfo)
            | (ResponseBody::Services(_), Operation::Services)
            | (
                ResponseBody::ServiceActionAccepted(..),
                Operation::ServiceAction
            )
            | (ResponseBody::ApCredentials(_), Operation::ApRead)
            | (ResponseBody::ApStored, Operation::ApWrite)
            | (
                ResponseBody::ApStarted(_),
                Operation::ApEnable | Operation::ApDisable
            )
    )
}

type SharedSession = Arc<Mutex<DeviceSessionState>>;
type WeakSession = Weak<Mutex<DeviceSessionState>>;
#[derive(Clone, Default)]
pub struct SessionRegistry {
    sessions: Arc<Mutex<Vec<WeakSession>>>,
    correlations: CorrelationBook,
}

impl SessionRegistry {
    fn attach(&self, state: &SharedSession) -> Result<(), SecureError> {
        let mut entries = self.sessions.lock().map_err(|_| SecureError::Closed)?;
        entries.retain(|weak| {
            weak.upgrade()
                .is_some_and(|entry| entry.lock().is_ok_and(|state| !state.channel.is_closed()))
        });
        let ip_sessions = entries
            .iter()
            .filter_map(|weak| weak.upgrade())
            .filter(|entry| entry.lock().is_ok_and(|state| state.over_ip))
            .count();
        let over_ip = state.lock().map_err(|_| SecureError::Closed)?.over_ip;
        if entries.len() >= MAX_SESSIONS || (over_ip && ip_sessions >= MAX_IP_SESSIONS) {
            return Err(SecureError::Limit);
        }
        entries.push(Arc::downgrade(state));
        Ok(())
    }
    /// The native runtime must poll this while waiting for transport input.
    pub fn poll_timeouts(&mut self, now: Duration) -> Result<(), SecureError> {
        let entries = self
            .sessions
            .lock()
            .map_err(|_| SecureError::Closed)?
            .clone();
        for entry in entries.into_iter().filter_map(|weak| weak.upgrade()) {
            let mut state = entry.lock().map_err(|_| SecureError::Closed)?;
            let unauthenticated_timeout = !state.authorized
                && now.saturating_sub(state.channel.started()) >= Duration::from_secs(10);
            if unauthenticated_timeout || state.channel.poll_timeout(now).is_err() {
                state.channel.close();
                state.authorized = false;
            }
        }
        Ok(())
    }
}
impl zorvinkel_application::registration::SessionControl for SessionRegistry {
    fn terminate(&mut self, client: &ClientKey) -> Result<(), ApplicationError> {
        let entries = self
            .sessions
            .lock()
            .map_err(|_| zorvinkel_application::state::StateError::Io)?
            .clone();
        for entry in entries.into_iter().filter_map(|weak| weak.upgrade()) {
            let mut state = entry
                .lock()
                .map_err(|_| zorvinkel_application::state::StateError::Io)?;
            if state.channel.peer_key().as_slice() == client.as_bytes() {
                state.channel.close();
                state.authorized = false;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        identity::IdentityHandle,
        security::{ChannelContext, handshake::Handshake, identity::generate_identity},
    };
    fn pair() -> (ClientSession, SecureChannel) {
        let a = IdentityHandle::from_material(generate_identity().unwrap());
        let b = IdentityHandle::from_material(generate_identity().unwrap());
        let mut client =
            Handshake::new(&a, Role::Client, ChannelContext::Simulation, Duration::ZERO).unwrap();
        let mut device =
            Handshake::new(&b, Role::Device, ChannelContext::Simulation, Duration::ZERO).unwrap();
        device
            .read(&client.write(Duration::ZERO).unwrap(), Duration::ZERO)
            .unwrap();
        client
            .read(&device.write(Duration::ZERO).unwrap(), Duration::ZERO)
            .unwrap();
        device
            .read(&client.write(Duration::ZERO).unwrap(), Duration::ZERO)
            .unwrap();
        (
            ClientSession::new(client.finish(Duration::ZERO).unwrap()).unwrap(),
            device.finish(Duration::ZERO).unwrap(),
        )
    }
    #[test]
    fn wrong_response_id_or_operation_closes_the_client() {
        for wrong_id in [true, false] {
            let (mut client, mut device) = pair();
            client.request(Operation::Register, Duration::ZERO).unwrap();
            let response = Response {
                id: if wrong_id { 2 } else { 1 },
                body: if wrong_id {
                    ResponseBody::Registered
                } else {
                    ResponseBody::Authenticated
                },
            };
            let record = device
                .encrypt(&encode_response(&response).unwrap(), Duration::ZERO)
                .unwrap();
            assert_eq!(
                client.receive(&record, Duration::ZERO),
                Err(SecureError::InvalidMessage)
            );
            assert_eq!(
                client.request(Operation::Status, Duration::ZERO),
                Err(SecureError::Closed)
            );
        }
    }
    #[test]
    fn manipulated_response_closes_the_client() {
        let (mut client, mut device) = pair();
        client.request(Operation::Register, Duration::ZERO).unwrap();
        let mut record = device
            .encrypt(
                &encode_response(&Response {
                    id: 1,
                    body: ResponseBody::Registered,
                })
                .unwrap(),
                Duration::ZERO,
            )
            .unwrap();
        *record.last_mut().unwrap() ^= 1;
        assert!(client.receive(&record, Duration::ZERO).is_err());
        assert_eq!(
            client.request(Operation::Status, Duration::ZERO),
            Err(SecureError::Closed)
        );
    }
}
