#![cfg(feature = "security")]
use std::time::Duration;
use zorvinkel_adapters::{
    identity::{IdentityHandle, IdentityMaterial},
    memory::{MemoryDeviceTrustStore, MemoryStateStore},
    security::{
        channel::*, handshake::*, identity::generate_identity, service::*, simulation::connect, *,
    },
    simulation::SimulatedNetwork,
};
use zorvinkel_application::{registration::*, state::*, trust::*};
use zorvinkel_core::{
    NetworkStatus,
    network::NetworkConfiguration,
    registry::{ClientKey, RegistrationState},
    trust::*,
};
use zorvinkel_protocol::status::*;
const NOW: Duration = Duration::ZERO;
fn identity() -> IdentityHandle {
    IdentityHandle::from_material(generate_identity().unwrap())
}
fn state() -> MemoryStateStore {
    MemoryStateStore::new(DeviceState::initial(NetworkConfiguration::Disconnected))
}
fn time(now: Duration) -> ServiceTime {
    ServiceTime {
        elapsed: now,
        registration_seconds: now.as_secs(),
    }
}
fn rpc(
    client: &mut ClientSession,
    device: &mut DeviceSession,
    store: &mut impl StateStore,
    operation: Operation,
) -> ResponseBody {
    let request = client.request(operation, NOW).unwrap();
    let response = device
        .handle(&request, store, &mut SimulatedNetwork::default(), time(NOW))
        .unwrap();
    client.receive(&response, NOW).unwrap().body
}
fn raw_pair(client: &IdentityHandle, device: &IdentityHandle) -> (SecureChannel, SecureChannel) {
    let mut client = Handshake::new(client, Role::Client, ChannelContext::Simulation, NOW).unwrap();
    let mut device = Handshake::new(device, Role::Device, ChannelContext::Simulation, NOW).unwrap();
    device.read(&client.write(NOW).unwrap(), NOW).unwrap();
    client.read(&device.write(NOW).unwrap(), NOW).unwrap();
    device.read(&client.write(NOW).unwrap(), NOW).unwrap();
    (client.finish(NOW).unwrap(), device.finish(NOW).unwrap())
}

#[test]
fn first_client_registers_then_reads_status_and_capabilities_and_reauthenticates() {
    let client_key = identity();
    let device_key = identity();
    let mut trust = MemoryDeviceTrustStore::default();
    let sessions = SessionRegistry::default();
    let mut store = state();
    let (mut client, mut device) = connect(
        &client_key,
        &device_key,
        &mut trust,
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        NOW,
        &sessions,
    )
    .unwrap();
    assert_eq!(
        rpc(&mut client, &mut device, &mut store, Operation::Register),
        ResponseBody::Registered
    );
    assert_eq!(
        rpc(&mut client, &mut device, &mut store, Operation::Status),
        ResponseBody::Status {
            mode: 0,
            ssid: String::new()
        }
    );
    assert_eq!(
        rpc(
            &mut client,
            &mut device,
            &mut store,
            Operation::Capabilities
        ),
        ResponseBody::Capabilities(CAPABILITIES)
    );
    assert_eq!(
        store.read().unwrap().registry.state,
        RegistrationState::Closed
    );
    drop((client, device));
    let (mut client, mut device) = connect(
        &client_key,
        &device_key,
        &mut trust,
        TrustMode::RequireKnownKey,
        ChannelContext::Simulation,
        NOW,
        &sessions,
    )
    .unwrap();
    assert_eq!(
        rpc(
            &mut client,
            &mut device,
            &mut store,
            Operation::Authenticate
        ),
        ResponseBody::Authenticated
    );
    assert!(matches!(
        rpc(&mut client, &mut device, &mut store, Operation::Status),
        ResponseBody::Status { .. }
    ));
    assert_eq!(store.read().unwrap().registry.clients.len(), 1);
}

#[test]
fn completed_handshake_alone_cannot_read_device_status() {
    let mut store = state();
    let mut trust = MemoryDeviceTrustStore::default();
    let sessions = SessionRegistry::default();
    let (mut client, mut device) = connect(
        &identity(),
        &identity(),
        &mut trust,
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        NOW,
        &sessions,
    )
    .unwrap();
    assert_eq!(
        rpc(&mut client, &mut device, &mut store, Operation::Status),
        ResponseBody::Error(RemoteError::Unauthorized)
    );
    assert!(device.is_closed());
    assert!(store.read().unwrap().registry.clients.is_empty());
}

#[test]
fn unknown_client_is_rejected_after_registration_closes() {
    let device_key = identity();
    let mut store = state();
    let sessions = SessionRegistry::default();
    let (mut client, mut device) = connect(
        &identity(),
        &device_key,
        &mut MemoryDeviceTrustStore::default(),
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        NOW,
        &sessions,
    )
    .unwrap();
    rpc(&mut client, &mut device, &mut store, Operation::Register);
    for operation in [Operation::Register, Operation::Authenticate] {
        let (mut unknown, mut device) = connect(
            &identity(),
            &device_key,
            &mut MemoryDeviceTrustStore::default(),
            TrustMode::TrustOnFirstUse,
            ChannelContext::Simulation,
            NOW,
            &sessions,
        )
        .unwrap();
        assert!(matches!(
            rpc(&mut unknown, &mut device, &mut store, operation),
            ResponseBody::Error(RemoteError::RegistrationClosed | RemoteError::Unauthorized)
        ));
        assert!(device.is_closed());
    }
    assert_eq!(store.read().unwrap().registry.clients.len(), 1);
}

#[test]
fn pinned_wrong_device_is_rejected_without_replacing_the_binding() {
    let expected = identity();
    let client_key = identity();
    let mut trust = MemoryDeviceTrustStore::default();
    configure_device(
        &mut trust,
        DeviceKey::new(expected.public_key().to_vec()).unwrap(),
    )
    .unwrap();
    let sessions = SessionRegistry::default();
    assert!(matches!(
        connect(
            &client_key,
            &identity(),
            &mut trust,
            TrustMode::TrustOnFirstUse,
            ChannelContext::Simulation,
            NOW,
            &sessions
        ),
        Err(SecureError::Trust)
    ));
    assert!(
        connect(
            &client_key,
            &expected,
            &mut trust,
            TrustMode::RequireKnownKey,
            ChannelContext::Simulation,
            NOW,
            &sessions
        )
        .is_ok()
    );
}

#[test]
fn setup_client_key_authenticates_without_open_registration() {
    let client_key = identity();
    let device_key = identity();
    let mut store = state();
    authorize_by_local_admin(
        &mut store,
        ClientKey::new(client_key.public_key().to_vec()).unwrap(),
    )
    .unwrap();
    let (mut client, mut device) = connect(
        &client_key,
        &device_key,
        &mut MemoryDeviceTrustStore::default(),
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        NOW,
        &SessionRegistry::default(),
    )
    .unwrap();
    assert_eq!(
        rpc(
            &mut client,
            &mut device,
            &mut store,
            Operation::Authenticate
        ),
        ResponseBody::Authenticated
    );
}

#[test]
fn lost_commit_response_is_recovered_by_a_new_authenticated_connection() {
    let client_key = identity();
    let device_key = identity();
    let mut store = state();
    let mut trust = MemoryDeviceTrustStore::default();
    let sessions = SessionRegistry::default();
    let (mut client, mut device) = connect(
        &client_key,
        &device_key,
        &mut trust,
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        NOW,
        &sessions,
    )
    .unwrap();
    let request = client.request(Operation::Register, NOW).unwrap();
    let _lost_response = device
        .handle(
            &request,
            &mut store,
            &mut SimulatedNetwork::default(),
            time(NOW),
        )
        .unwrap();
    assert_eq!(
        client.request(Operation::Register, NOW),
        Err(SecureError::Busy)
    );
    drop((client, device));
    let (mut client, mut device) = connect(
        &client_key,
        &device_key,
        &mut trust,
        TrustMode::RequireKnownKey,
        ChannelContext::Simulation,
        NOW,
        &sessions,
    )
    .unwrap();
    assert_eq!(
        rpc(
            &mut client,
            &mut device,
            &mut store,
            Operation::Authenticate
        ),
        ResponseBody::Authenticated
    );
    assert_eq!(store.read().unwrap().registry.clients.len(), 1);
}

#[test]
fn manipulated_ciphertext_and_replay_close_the_session_before_dispatch() {
    for manipulate in [true, false] {
        let mut store = state();
        let (mut client, mut device) = connect(
            &identity(),
            &identity(),
            &mut MemoryDeviceTrustStore::default(),
            TrustMode::TrustOnFirstUse,
            ChannelContext::Simulation,
            NOW,
            &SessionRegistry::default(),
        )
        .unwrap();
        let mut request = client.request(Operation::Register, NOW).unwrap();
        if manipulate {
            *request.last_mut().unwrap() ^= 1;
        } else {
            device
                .handle(
                    &request,
                    &mut store,
                    &mut SimulatedNetwork::default(),
                    time(NOW),
                )
                .unwrap();
        }
        assert!(
            device
                .handle(
                    &request,
                    &mut store,
                    &mut SimulatedNetwork::default(),
                    time(NOW)
                )
                .is_err()
        );
        assert!(device.is_closed());
        assert_eq!(
            store.read().unwrap().registry.clients.len(),
            usize::from(!manipulate)
        );
    }
}

#[test]
fn old_ciphertext_is_invalid_in_a_fresh_session_with_the_same_identities() {
    let client_key = identity();
    let device_key = identity();
    let mut store = state();
    let sessions = SessionRegistry::default();
    let mut trust = MemoryDeviceTrustStore::default();
    let (mut old_client, old_device) = connect(
        &client_key,
        &device_key,
        &mut trust,
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        NOW,
        &sessions,
    )
    .unwrap();
    let old_request = old_client.request(Operation::Register, NOW).unwrap();
    drop((old_client, old_device));
    let (_, mut fresh_device) = connect(
        &client_key,
        &device_key,
        &mut trust,
        TrustMode::RequireKnownKey,
        ChannelContext::Simulation,
        NOW,
        &sessions,
    )
    .unwrap();
    assert!(
        fresh_device
            .handle(
                &old_request,
                &mut store,
                &mut SimulatedNetwork::default(),
                time(NOW)
            )
            .is_err()
    );
    assert!(store.read().unwrap().registry.clients.is_empty());
}

#[test]
fn revocation_terminates_active_sessions_and_does_not_reopen_registration() {
    let client_key = identity();
    let device_key = identity();
    let mut store = state();
    let mut sessions = SessionRegistry::default();
    let (mut client, mut device) = connect(
        &client_key,
        &device_key,
        &mut MemoryDeviceTrustStore::default(),
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        NOW,
        &sessions,
    )
    .unwrap();
    rpc(&mut client, &mut device, &mut store, Operation::Register);
    let actor = ClientKey::new(client_key.public_key().to_vec()).unwrap();
    revoke_with_sessions(&mut store, &mut sessions, &actor, &actor).unwrap();
    assert!(device.is_closed());
    assert_eq!(
        store.read().unwrap().registry.state,
        RegistrationState::Closed
    );
    let request = client.request(Operation::Status, NOW).unwrap();
    assert_eq!(
        device.handle(
            &request,
            &mut store,
            &mut SimulatedNetwork::default(),
            time(NOW)
        ),
        Err(SecureError::Closed)
    );
}

#[test]
fn malformed_versions_ids_and_operations_never_bypass_authorization() {
    for change_version in [true, false] {
        let (mut client, server) = raw_pair(&identity(), &identity());
        let mut device = DeviceSession::new(server, &SessionRegistry::default()).unwrap();
        let mut store = state();
        let mut bytes = encode_request(Request {
            id: if change_version { 1 } else { 2 },
            operation: Operation::Register,
        })
        .unwrap();
        if change_version {
            bytes[2] = 2;
        }
        let request = client.encrypt(&bytes, NOW).unwrap();
        assert!(
            device
                .handle(
                    &request,
                    &mut store,
                    &mut SimulatedNetwork::default(),
                    time(NOW)
                )
                .is_err()
        );
        assert!(store.read().unwrap().registry.clients.is_empty());
    }
    let mut store = state();
    let (mut client, mut device) = connect(
        &identity(),
        &identity(),
        &mut MemoryDeviceTrustStore::default(),
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        NOW,
        &SessionRegistry::default(),
    )
    .unwrap();
    rpc(&mut client, &mut device, &mut store, Operation::Register);
    assert_eq!(
        rpc(&mut client, &mut device, &mut store, Operation::Unknown(99)),
        ResponseBody::Error(RemoteError::Unsupported)
    );
    assert!(matches!(
        rpc(&mut client, &mut device, &mut store, Operation::Status),
        ResponseBody::Status { .. }
    ));
}

#[test]
fn handshake_deadline_context_and_invalid_identity_fail_closed() {
    let client_key = identity();
    let device_key = identity();
    let mut client = Handshake::new(&client_key, Role::Client, ChannelContext::Ble, NOW).unwrap();
    let mut device = Handshake::new(&device_key, Role::Device, ChannelContext::Ip, NOW).unwrap();
    device.read(&client.write(NOW).unwrap(), NOW).unwrap();
    assert!(client.read(&device.write(NOW).unwrap(), NOW).is_err());
    assert_eq!(client.write(NOW), Err(SecureError::Closed));
    let mut client =
        Handshake::new(&client_key, Role::Client, ChannelContext::Simulation, NOW).unwrap();
    assert_eq!(client.write(HANDSHAKE_TIMEOUT), Err(SecureError::TimedOut));
    assert_eq!(client.write(HANDSHAKE_TIMEOUT), Err(SecureError::Closed));
    let invalid = IdentityHandle::from_material(
        IdentityMaterial::new(device_key.public_key().to_vec(), vec![0; 32]).unwrap(),
    );
    assert!(matches!(
        Handshake::new(&invalid, Role::Client, ChannelContext::Simulation, NOW),
        Err(SecureError::InvalidIdentity)
    ));
}

#[test]
fn channel_limits_and_absolute_lifetime_require_a_new_handshake() {
    let (mut client, mut device) = raw_pair(&identity(), &identity());
    let payload = vec![7; MAX_PLAINTEXT];
    let record = client.encrypt(&payload, NOW).unwrap();
    assert_eq!(record.len(), MAX_RECORD);
    assert_eq!(&device.decrypt(&record, NOW).unwrap()[..], payload);
    for _ in 1..MAX_RECORDS {
        let record = client.encrypt(b"message", NOW).unwrap();
        device.decrypt(&record, NOW).unwrap();
    }
    assert_eq!(client.encrypt(b"message", NOW), Err(SecureError::Limit));
    let (mut client, _) = raw_pair(&identity(), &identity());
    assert_eq!(
        client.encrypt(b"message", SESSION_LIFETIME),
        Err(SecureError::TimedOut)
    );
    assert_eq!(
        client.encrypt(b"message", SESSION_LIFETIME),
        Err(SecureError::Closed)
    );
}

#[test]
fn inactive_unauthorized_sessions_are_bounded_and_expire() {
    let device_key = identity();
    let mut sessions = SessionRegistry::default();
    let mut active = Vec::new();
    for _ in 0..4 {
        active.push(
            connect(
                &identity(),
                &device_key,
                &mut MemoryDeviceTrustStore::default(),
                TrustMode::TrustOnFirstUse,
                ChannelContext::Simulation,
                NOW,
                &sessions,
            )
            .unwrap(),
        );
    }
    assert!(matches!(
        connect(
            &identity(),
            &device_key,
            &mut MemoryDeviceTrustStore::default(),
            TrustMode::TrustOnFirstUse,
            ChannelContext::Simulation,
            NOW,
            &sessions
        ),
        Err(SecureError::Limit)
    ));
    sessions.poll_timeouts(Duration::from_secs(10)).unwrap();
    assert!(active.iter().all(|(_, device)| device.is_closed()));
    assert!(
        connect(
            &identity(),
            &device_key,
            &mut MemoryDeviceTrustStore::default(),
            TrustMode::TrustOnFirstUse,
            ChannelContext::Simulation,
            Duration::from_secs(10),
            &sessions
        )
        .is_ok()
    );
}

#[test]
fn status_is_observed_only_after_authorization_and_never_confirms_ip_reachability() {
    struct Network {
        reads: usize,
    }
    impl zorvinkel_application::device::NetworkStatusSource for Network {
        fn read_status(&mut self) -> Result<NetworkStatus, zorvinkel_application::PortError> {
            self.reads += 1;
            Ok(NetworkStatus::WifiConnected {
                ssid: "Test network".into(),
            })
        }
    }
    let mut network = Network { reads: 0 };
    let mut store = state();
    let sessions = SessionRegistry::default();
    let (mut client, mut device) = connect(
        &identity(),
        &identity(),
        &mut MemoryDeviceTrustStore::default(),
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        NOW,
        &sessions,
    )
    .unwrap();
    let record = client.request(Operation::Status, NOW).unwrap();
    let reply = device
        .handle(&record, &mut store, &mut network, time(NOW))
        .unwrap();
    assert_eq!(
        client.receive(&reply, NOW).unwrap().body,
        ResponseBody::Error(RemoteError::Unauthorized)
    );
    assert_eq!(network.reads, 0);
    drop((client, device));
    let (mut client, mut device) = connect(
        &identity(),
        &identity(),
        &mut MemoryDeviceTrustStore::default(),
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        NOW,
        &sessions,
    )
    .unwrap();
    rpc(&mut client, &mut device, &mut store, Operation::Register);
    let record = client.request(Operation::Status, NOW).unwrap();
    let reply = device
        .handle(&record, &mut store, &mut network, time(NOW))
        .unwrap();
    assert_eq!(
        client.receive(&reply, NOW).unwrap().body,
        ResponseBody::Status {
            mode: 1,
            ssid: "Test network".into()
        }
    );
    assert_eq!(network.reads, 1);
}

#[test]
fn registration_commit_failure_cannot_authorize_a_status_request() {
    struct FailedStore(MemoryStateStore);
    impl StateStore for FailedStore {
        fn read(&mut self) -> Result<DeviceState, StateError> {
            self.0.read()
        }
        fn update<T>(
            &mut self,
            _: impl FnOnce(&mut DeviceState) -> Result<T, ApplicationError>,
        ) -> Result<T, ApplicationError> {
            Err(StateError::Io.into())
        }
    }
    let mut store = FailedStore(state());
    let (mut client, mut device) = connect(
        &identity(),
        &identity(),
        &mut MemoryDeviceTrustStore::default(),
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        NOW,
        &SessionRegistry::default(),
    )
    .unwrap();
    assert_eq!(
        rpc(&mut client, &mut device, &mut store, Operation::Register),
        ResponseBody::Error(RemoteError::Unavailable)
    );
    assert_eq!(
        rpc(&mut client, &mut device, &mut store, Operation::Status),
        ResponseBody::Error(RemoteError::Unauthorized)
    );
    assert!(store.read().unwrap().registry.clients.is_empty());
}

#[test]
fn concurrent_encrypted_first_registrations_have_exactly_one_winner() {
    let store = state();
    let sessions = SessionRegistry::default();
    let device_key = identity();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let (mut client, mut device) = connect(
            &identity(),
            &device_key,
            &mut MemoryDeviceTrustStore::default(),
            TrustMode::TrustOnFirstUse,
            ChannelContext::Simulation,
            NOW,
            &sessions,
        )
        .unwrap();
        let mut store = store.clone();
        let barrier = barrier.clone();
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            rpc(&mut client, &mut device, &mut store, Operation::Register)
        }));
    }
    let replies: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(
        replies
            .iter()
            .filter(|reply| **reply == ResponseBody::Registered)
            .count(),
        1
    );
    assert_eq!(
        replies
            .iter()
            .filter(|reply| **reply == ResponseBody::Error(RemoteError::RegistrationClosed))
            .count(),
        1
    );
}

#[test]
fn local_recovery_grants_only_the_intended_key_after_all_clients_are_revoked() {
    let old_client = identity();
    let new_client = identity();
    let device_key = identity();
    let mut store = state();
    let mut sessions = SessionRegistry::default();
    let (mut client, mut device) = connect(
        &old_client,
        &device_key,
        &mut MemoryDeviceTrustStore::default(),
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        NOW,
        &sessions,
    )
    .unwrap();
    rpc(&mut client, &mut device, &mut store, Operation::Register);
    let actor = ClientKey::new(old_client.public_key().to_vec()).unwrap();
    revoke_with_sessions(&mut store, &mut sessions, &actor, &actor).unwrap();
    grant_by_local_admin(
        &mut store,
        ClientKey::new(new_client.public_key().to_vec()).unwrap(),
        0,
        10,
    )
    .unwrap();
    let (mut stranger, mut stranger_device) = connect(
        &identity(),
        &device_key,
        &mut MemoryDeviceTrustStore::default(),
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        NOW,
        &sessions,
    )
    .unwrap();
    assert_eq!(
        rpc(
            &mut stranger,
            &mut stranger_device,
            &mut store,
            Operation::Register
        ),
        ResponseBody::Error(RemoteError::RegistrationClosed)
    );
    let (mut client, mut device) = connect(
        &new_client,
        &device_key,
        &mut MemoryDeviceTrustStore::default(),
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        NOW,
        &sessions,
    )
    .unwrap();
    assert_eq!(
        rpc(&mut client, &mut device, &mut store, Operation::Register),
        ResponseBody::Registered
    );
    assert_eq!(
        store.read().unwrap().registry.state,
        RegistrationState::Closed
    );
}

#[test]
fn broken_trust_storage_prevents_registration_before_any_application_request() {
    struct BrokenTrust;
    impl DeviceTrustStore for BrokenTrust {
        fn update<T>(
            &mut self,
            _: impl FnOnce(&mut DeviceTrust) -> Result<T, ApplicationError>,
        ) -> Result<T, ApplicationError> {
            Err(StateError::Corrupt.into())
        }
    }
    assert!(matches!(
        connect(
            &identity(),
            &identity(),
            &mut BrokenTrust,
            TrustMode::TrustOnFirstUse,
            ChannelContext::Simulation,
            NOW,
            &SessionRegistry::default()
        ),
        Err(SecureError::Storage)
    ));
}

#[test]
fn truncated_handshake_and_clock_reversal_close_the_handshake_and_transport() {
    let mut client =
        Handshake::new(&identity(), Role::Client, ChannelContext::Simulation, NOW).unwrap();
    let mut device =
        Handshake::new(&identity(), Role::Device, ChannelContext::Simulation, NOW).unwrap();
    let mut record = client.write(NOW).unwrap();
    record.pop();
    assert!(device.read(&record, NOW).is_err());
    assert_eq!(device.read(&record, NOW), Err(SecureError::Closed));
    let mut client = Handshake::new(
        &identity(),
        Role::Client,
        ChannelContext::Simulation,
        Duration::from_secs(1),
    )
    .unwrap();
    assert_eq!(client.write(NOW), Err(SecureError::ClockMovedBackwards));
    let (mut client, _) = raw_pair(&identity(), &identity());
    client.encrypt(b"message", Duration::from_secs(1)).unwrap();
    assert_eq!(
        client.encrypt(b"message", NOW),
        Err(SecureError::ClockMovedBackwards)
    );
    assert_eq!(client.encrypt(b"message", NOW), Err(SecureError::Closed));
}

#[test]
fn wifi_commands_are_encrypted_authorized_and_available_only_when_advertised() {
    use zorvinkel_application::{device::NetworkStatusSource, wifi::*};
    use zorvinkel_protocol::wifi::*;
    struct Wifi {
        calls: usize,
    }
    impl NetworkStatusSource for Wifi {
        fn read_status(&mut self) -> Result<NetworkStatus, zorvinkel_application::PortError> {
            Ok(NetworkStatus::Disconnected)
        }
        fn wifi_setup(&mut self) -> Option<&mut dyn WifiSetupControl> {
            Some(self)
        }
    }
    impl WifiSetupControl for Wifi {
        fn scan(&mut self, _: &ClientKey, _: u64) -> Result<(), WifiError> {
            self.calls += 1;
            Ok(())
        }
        fn connect(
            &mut self,
            _: &ClientKey,
            _: u64,
            ssid: &str,
            password: &str,
        ) -> Result<(), WifiError> {
            assert_eq!((ssid, password), ("Event", "test-password"));
            self.calls += 1;
            Ok(())
        }
        fn poll(&mut self, _: &ClientKey, _: u64) -> Result<WifiTaskState, WifiError> {
            Ok(WifiTaskState::Connected)
        }
    }
    fn call(
        client: &mut ClientSession,
        device: &mut DeviceSession,
        store: &mut MemoryStateStore,
        wifi: &mut Wifi,
        op: Operation,
        payload: &[u8],
    ) -> ResponseBody {
        let record = client.request_payload(op, payload, NOW).unwrap();
        assert!(
            !record
                .windows(b"test-password".len())
                .any(|part| part == b"test-password")
        );
        let reply = device.handle(&record, store, wifi, time(NOW)).unwrap();
        client.receive(&reply, NOW).unwrap().body
    }
    let client_key = identity();
    let device_key = identity();
    let sessions = SessionRegistry::default();
    let pair = || {
        connect(
            &client_key,
            &device_key,
            &mut MemoryDeviceTrustStore::default(),
            TrustMode::TrustOnFirstUse,
            ChannelContext::Simulation,
            NOW,
            &sessions,
        )
        .unwrap()
    };
    let mut store = state();
    let mut wifi = Wifi { calls: 0 };
    let (mut client, mut device) = pair();
    let credentials = connect_payload(91, "Event", "test-password").unwrap();
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::WifiConnect,
            &credentials
        ),
        ResponseBody::Error(RemoteError::Unauthorized)
    );
    assert_eq!(wifi.calls, 0);
    let (mut client, mut device) = pair();
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::Register,
            &[]
        ),
        ResponseBody::Registered
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::Capabilities,
            &[]
        ),
        ResponseBody::Capabilities(CAPABILITIES | CAP_WIFI_TEMPORARY)
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::WifiScan,
            &id_payload(90).unwrap()
        ),
        ResponseBody::WifiScanStarted(90)
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::WifiConnect,
            &credentials
        ),
        ResponseBody::WifiConnectStarted(91)
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::WifiPoll,
            &id_payload(91).unwrap()
        ),
        ResponseBody::WifiTask(WifiTask {
            id: 91,
            state: 2,
            networks: vec![]
        })
    );
    assert_eq!(wifi.calls, 2);
}

#[test]
fn access_point_commands_are_encrypted_authorized_and_offered_only_when_supported() {
    use zorvinkel_application::{device::NetworkStatusSource, setup::AccessPointSettings, wifi::*};
    use zorvinkel_protocol::ap::*;
    use zorvinkel_protocol::wifi::id_payload;
    const PASSWORD: &str = "Zk3pQw8LmN2xVb5RtY7uHs9DfG1jCe4AaBc6MoIn0KlPqWrEtYuiOpSdFgHjKlZ";
    #[derive(Default)]
    struct Wifi {
        supported: bool,
        stored: Option<(String, String)>,
        switched: Vec<(u64, bool)>,
    }
    impl NetworkStatusSource for Wifi {
        fn read_status(&mut self) -> Result<NetworkStatus, zorvinkel_application::PortError> {
            Ok(NetworkStatus::Disconnected)
        }
        fn wifi_setup(&mut self) -> Option<&mut dyn WifiSetupControl> {
            Some(self)
        }
    }
    impl WifiSetupControl for Wifi {
        fn scan(&mut self, _: &ClientKey, _: u64) -> Result<(), WifiError> {
            Err(WifiError::Unsupported)
        }
        fn connect(&mut self, _: &ClientKey, _: u64, _: &str, _: &str) -> Result<(), WifiError> {
            Err(WifiError::Unsupported)
        }
        fn poll(&mut self, _: &ClientKey, _: u64) -> Result<WifiTaskState, WifiError> {
            Ok(WifiTaskState::Connected)
        }
        fn access_point_supported(&self) -> bool {
            self.supported
        }
        fn access_point(
            &mut self,
            _: &ClientKey,
        ) -> Result<Option<AccessPointSettings>, WifiError> {
            Ok(self
                .stored
                .clone()
                .map(|(ssid, password)| AccessPointSettings { ssid, password }))
        }
        fn set_access_point(
            &mut self,
            _: &ClientKey,
            ssid: &str,
            password: &str,
        ) -> Result<(), WifiError> {
            self.stored = Some((ssid.into(), password.into()));
            Ok(())
        }
        fn switch_access_point(
            &mut self,
            _: &ClientKey,
            id: u64,
            enabled: bool,
        ) -> Result<(), WifiError> {
            self.switched.push((id, enabled));
            Ok(())
        }
    }
    fn call(
        client: &mut ClientSession,
        device: &mut DeviceSession,
        store: &mut MemoryStateStore,
        wifi: &mut Wifi,
        op: Operation,
        payload: &[u8],
    ) -> ResponseBody {
        let record = client.request_payload(op, payload, NOW).unwrap();
        let reply = device.handle(&record, store, wifi, time(NOW)).unwrap();
        client.receive(&reply, NOW).unwrap().body
    }
    let client_key = identity();
    let device_key = identity();
    let sessions = SessionRegistry::default();
    let pair = || {
        connect(
            &client_key,
            &device_key,
            &mut MemoryDeviceTrustStore::default(),
            TrustMode::TrustOnFirstUse,
            ChannelContext::Simulation,
            NOW,
            &sessions,
        )
        .unwrap()
    };
    let mut store = state();
    let mut wifi = Wifi {
        supported: true,
        ..Default::default()
    };
    let credentials = credentials_payload("Zorvinkel-Box", PASSWORD).unwrap();
    // Without registration nothing is read or changed.
    let (mut client, mut device) = pair();
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::ApWrite,
            &credentials
        ),
        ResponseBody::Error(RemoteError::Unauthorized)
    );
    assert!(wifi.stored.is_none());
    let (mut client, mut device) = pair();
    call(
        &mut client,
        &mut device,
        &mut store,
        &mut wifi,
        Operation::Register,
        &[],
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::Capabilities,
            &[]
        ),
        ResponseBody::Capabilities(
            CAPABILITIES | zorvinkel_protocol::wifi::CAP_WIFI_TEMPORARY | CAP_ACCESS_POINT
        )
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::ApRead,
            &[]
        ),
        ResponseBody::ApCredentials(None)
    );
    // The passphrase never travels in clear text.
    let record = client
        .request_payload(Operation::ApWrite, &credentials, NOW)
        .unwrap();
    assert!(
        !record
            .windows(PASSWORD.len())
            .any(|part| part == PASSWORD.as_bytes())
    );
    let reply = device
        .handle(&record, &mut store, &mut wifi, time(NOW))
        .unwrap();
    assert_eq!(
        client.receive(&reply, NOW).unwrap().body,
        ResponseBody::ApStored
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::ApRead,
            &[]
        ),
        ResponseBody::ApCredentials(Some(ApCredentials {
            ssid: "Zorvinkel-Box".into(),
            password: PASSWORD.into()
        }))
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::ApEnable,
            &id_payload(5).unwrap()
        ),
        ResponseBody::ApStarted(5)
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::ApDisable,
            &id_payload(6).unwrap()
        ),
        ResponseBody::ApStarted(6)
    );
    assert_eq!(wifi.switched, [(5, true), (6, false)]);

    // A device without the helper does not advertise the feature and refuses every command.
    let mut without = Wifi::default();
    let (mut client, mut device) = pair();
    call(
        &mut client,
        &mut device,
        &mut store,
        &mut without,
        Operation::Authenticate,
        &[],
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut without,
            Operation::Capabilities,
            &[]
        ),
        ResponseBody::Capabilities(CAPABILITIES | zorvinkel_protocol::wifi::CAP_WIFI_TEMPORARY)
    );
    for (operation, payload) in [
        (Operation::ApRead, vec![]),
        (Operation::ApWrite, credentials.clone()),
        (Operation::ApEnable, id_payload(7).unwrap()),
    ] {
        assert_eq!(
            call(
                &mut client,
                &mut device,
                &mut store,
                &mut without,
                operation,
                &payload
            ),
            ResponseBody::Error(RemoteError::Unsupported)
        );
    }
    assert!(without.stored.is_none() && without.switched.is_empty());
}

#[test]
fn power_commands_need_authorization_and_are_offered_only_when_the_device_supports_them() {
    use zorvinkel_application::{
        device::NetworkStatusSource,
        power::{PowerAction, PowerControl, PowerError},
    };
    use zorvinkel_protocol::power::*;
    #[derive(Default)]
    struct Power {
        supported: bool,
        scheduled: Vec<PowerAction>,
    }
    impl NetworkStatusSource for Power {
        fn read_status(&mut self) -> Result<NetworkStatus, zorvinkel_application::PortError> {
            Ok(NetworkStatus::Disconnected)
        }
        fn power_control(&mut self) -> Option<&mut dyn PowerControl> {
            self.supported.then_some(self as &mut dyn PowerControl)
        }
    }
    impl PowerControl for Power {
        fn schedule(&mut self, action: PowerAction) -> Result<(), PowerError> {
            if self.scheduled.is_empty() {
                self.scheduled.push(action);
                Ok(())
            } else {
                Err(PowerError::Busy)
            }
        }
    }
    fn call(
        client: &mut ClientSession,
        device: &mut DeviceSession,
        store: &mut MemoryStateStore,
        power: &mut Power,
        operation: Operation,
        payload: &[u8],
    ) -> ResponseBody {
        let record = client.request_payload(operation, payload, NOW).unwrap();
        let reply = device.handle(&record, store, power, time(NOW)).unwrap();
        client.receive(&reply, NOW).unwrap().body
    }
    let (client_key, device_key) = (identity(), identity());
    let sessions = SessionRegistry::default();
    let pair = || {
        connect(
            &client_key,
            &device_key,
            &mut MemoryDeviceTrustStore::default(),
            TrustMode::TrustOnFirstUse,
            ChannelContext::Simulation,
            NOW,
            &sessions,
        )
        .unwrap()
    };
    let mut store = state();
    let mut power = Power {
        supported: true,
        ..Default::default()
    };
    let shutdown = power_payload(PowerCommand::Shutdown);
    let (mut client, mut device) = pair();
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut power,
            Operation::Power,
            &shutdown
        ),
        ResponseBody::Error(RemoteError::Unauthorized)
    );
    assert!(power.scheduled.is_empty());
    let (mut client, mut device) = pair();
    call(
        &mut client,
        &mut device,
        &mut store,
        &mut power,
        Operation::Register,
        &[],
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut power,
            Operation::Capabilities,
            &[]
        ),
        ResponseBody::Capabilities(CAPABILITIES | CAP_POWER)
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut power,
            Operation::Power,
            &shutdown
        ),
        ResponseBody::PowerAccepted(PowerCommand::Shutdown)
    );
    assert_eq!(power.scheduled, [PowerAction::Shutdown]);
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut power,
            Operation::Power,
            &power_payload(PowerCommand::Restart)
        ),
        ResponseBody::Error(RemoteError::Busy)
    );
    power.supported = false;
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut power,
            Operation::Capabilities,
            &[]
        ),
        ResponseBody::Capabilities(CAPABILITIES)
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut power,
            Operation::Power,
            &shutdown
        ),
        ResponseBody::Error(RemoteError::Unsupported)
    );
}

#[test]
fn system_information_needs_authorization_and_is_offered_only_when_available() {
    use zorvinkel_application::{
        PortError,
        device::NetworkStatusSource,
        system::{SystemInfo, SystemInfoSource},
    };
    use zorvinkel_protocol::system::CAP_SYSTEM_INFO;
    struct Source {
        supported: bool,
        failing: bool,
    }
    impl NetworkStatusSource for Source {
        fn read_status(&mut self) -> Result<NetworkStatus, PortError> {
            Ok(NetworkStatus::Disconnected)
        }
        fn system_info(&mut self) -> Option<&mut dyn SystemInfoSource> {
            self.supported.then_some(self as &mut dyn SystemInfoSource)
        }
    }
    impl SystemInfoSource for Source {
        fn read(&mut self) -> Result<SystemInfo, PortError> {
            if self.failing {
                return Err(PortError::Unavailable);
            }
            Ok(SystemInfo {
                version: "0.1.0".into(),
                uptime_seconds: 60,
                load_milli: [10, 20, 30],
                memory_total: 1000,
                memory_available: 400,
                storage_total: 5000,
                storage_free: 2000,
                temperature_milli: None,
                cpu_mhz: None,
                cpu_max_mhz: None,
                throttle_flags: None,
                os_name: None,
                kernel: None,
                model: None,
                hostname: Some("Box".into()),
                addresses: Vec::new(),
            })
        }
    }
    fn call(
        client: &mut ClientSession,
        device: &mut DeviceSession,
        store: &mut MemoryStateStore,
        source: &mut Source,
        operation: Operation,
    ) -> ResponseBody {
        let record = client.request_payload(operation, &[], NOW).unwrap();
        let reply = device.handle(&record, store, source, time(NOW)).unwrap();
        client.receive(&reply, NOW).unwrap().body
    }
    let (client_key, device_key) = (identity(), identity());
    let sessions = SessionRegistry::default();
    let pair = || {
        connect(
            &client_key,
            &device_key,
            &mut MemoryDeviceTrustStore::default(),
            TrustMode::TrustOnFirstUse,
            ChannelContext::Simulation,
            NOW,
            &sessions,
        )
        .unwrap()
    };
    let mut store = state();
    let mut source = Source {
        supported: true,
        failing: false,
    };
    let (mut client, mut device) = pair();
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut source,
            Operation::SystemInfo
        ),
        ResponseBody::Error(RemoteError::Unauthorized)
    );
    let (mut client, mut device) = pair();
    call(
        &mut client,
        &mut device,
        &mut store,
        &mut source,
        Operation::Register,
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut source,
            Operation::Capabilities
        ),
        ResponseBody::Capabilities(CAPABILITIES | CAP_SYSTEM_INFO)
    );
    assert!(matches!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut source,
            Operation::SystemInfo
        ),
        ResponseBody::SystemInfo(info) if info.memory_available == 400 && info.load_milli == [10, 20, 30]
    ));
    source.failing = true;
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut source,
            Operation::SystemInfo
        ),
        ResponseBody::Error(RemoteError::Unavailable)
    );
    source.supported = false;
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut source,
            Operation::Capabilities
        ),
        ResponseBody::Capabilities(CAPABILITIES)
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut source,
            Operation::SystemInfo
        ),
        ResponseBody::Error(RemoteError::Unsupported)
    );
}

#[test]
fn service_list_needs_authorization_and_is_offered_only_when_configured() {
    use zorvinkel_application::{
        PortError,
        device::NetworkStatusSource,
        services::{ServiceEntry, ServiceInfoSource, ServiceState},
    };
    use zorvinkel_protocol::services::CAP_SERVICES;
    struct Source {
        supported: bool,
        failing: bool,
    }
    impl NetworkStatusSource for Source {
        fn read_status(&mut self) -> Result<NetworkStatus, PortError> {
            Ok(NetworkStatus::Disconnected)
        }
        fn service_info(&mut self) -> Option<&mut dyn ServiceInfoSource> {
            self.supported.then_some(self as &mut dyn ServiceInfoSource)
        }
    }
    impl ServiceInfoSource for Source {
        fn read(&mut self) -> Result<Vec<ServiceEntry>, PortError> {
            if self.failing {
                return Err(PortError::Unavailable);
            }
            Ok(vec![ServiceEntry {
                group: "Musik".into(),
                name: "Raspotify".into(),
                version: Some("0.48.1".into()),
                state: ServiceState::Active,
                actions: Vec::new(),
                web: None,
            }])
        }
    }
    fn call(
        client: &mut ClientSession,
        device: &mut DeviceSession,
        store: &mut MemoryStateStore,
        source: &mut Source,
        operation: Operation,
    ) -> ResponseBody {
        let record = client.request_payload(operation, &[], NOW).unwrap();
        let reply = device.handle(&record, store, source, time(NOW)).unwrap();
        client.receive(&reply, NOW).unwrap().body
    }
    let (client_key, device_key) = (identity(), identity());
    let sessions = SessionRegistry::default();
    let pair = || {
        connect(
            &client_key,
            &device_key,
            &mut MemoryDeviceTrustStore::default(),
            TrustMode::TrustOnFirstUse,
            ChannelContext::Simulation,
            NOW,
            &sessions,
        )
        .unwrap()
    };
    let mut store = state();
    let mut source = Source {
        supported: true,
        failing: false,
    };
    let (mut client, mut device) = pair();
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut source,
            Operation::Services
        ),
        ResponseBody::Error(RemoteError::Unauthorized)
    );
    let (mut client, mut device) = pair();
    call(
        &mut client,
        &mut device,
        &mut store,
        &mut source,
        Operation::Register,
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut source,
            Operation::Capabilities
        ),
        ResponseBody::Capabilities(CAPABILITIES | CAP_SERVICES)
    );
    assert!(matches!(
        call(&mut client, &mut device, &mut store, &mut source, Operation::Services),
        ResponseBody::Services(services)
            if services.len() == 1 && services[0].name == "Raspotify"
                && services[0].version.as_deref() == Some("0.48.1")
    ));
    source.failing = true;
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut source,
            Operation::Services
        ),
        ResponseBody::Error(RemoteError::Unavailable)
    );
    source.supported = false;
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut source,
            Operation::Capabilities
        ),
        ResponseBody::Capabilities(CAPABILITIES)
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut source,
            Operation::Services
        ),
        ResponseBody::Error(RemoteError::Unsupported)
    );
}

#[test]
fn service_actions_need_authorization_and_stay_within_the_configuration() {
    use zorvinkel_application::{
        PortError,
        device::NetworkStatusSource,
        services::{ServiceAction, ServiceControl, ServiceControlError},
    };
    use zorvinkel_protocol::services::{self, CAP_SERVICE_ACTIONS};
    struct Control {
        supported: bool,
        result: Result<(), ServiceControlError>,
        performed: Vec<(String, ServiceAction)>,
    }
    impl NetworkStatusSource for Control {
        fn read_status(&mut self) -> Result<NetworkStatus, PortError> {
            Ok(NetworkStatus::Disconnected)
        }
        fn service_control(&mut self) -> Option<&mut dyn ServiceControl> {
            self.supported.then_some(self as &mut dyn ServiceControl)
        }
    }
    impl ServiceControl for Control {
        fn perform(
            &mut self,
            name: &str,
            action: ServiceAction,
        ) -> Result<(), ServiceControlError> {
            self.performed.push((name.into(), action));
            self.result
        }
    }
    fn call(
        client: &mut ClientSession,
        device: &mut DeviceSession,
        store: &mut MemoryStateStore,
        control: &mut Control,
        operation: Operation,
        payload: &[u8],
    ) -> ResponseBody {
        let record = client.request_payload(operation, payload, NOW).unwrap();
        let reply = device.handle(&record, store, control, time(NOW)).unwrap();
        client.receive(&reply, NOW).unwrap().body
    }
    let (client_key, device_key) = (identity(), identity());
    let sessions = SessionRegistry::default();
    let pair = || {
        connect(
            &client_key,
            &device_key,
            &mut MemoryDeviceTrustStore::default(),
            TrustMode::TrustOnFirstUse,
            ChannelContext::Simulation,
            NOW,
            &sessions,
        )
        .unwrap()
    };
    let payload = services::action_payload(services::ServiceAction::Restart, "Raspotify").unwrap();
    let mut store = state();
    let mut control = Control {
        supported: true,
        result: Ok(()),
        performed: Vec::new(),
    };
    let (mut client, mut device) = pair();
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut control,
            Operation::ServiceAction,
            &payload
        ),
        ResponseBody::Error(RemoteError::Unauthorized)
    );
    assert!(control.performed.is_empty());
    let (mut client, mut device) = pair();
    call(
        &mut client,
        &mut device,
        &mut store,
        &mut control,
        Operation::Register,
        &[],
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut control,
            Operation::Capabilities,
            &[]
        ),
        ResponseBody::Capabilities(CAPABILITIES | CAP_SERVICE_ACTIONS)
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut control,
            Operation::ServiceAction,
            &payload
        ),
        ResponseBody::ServiceActionAccepted(services::ServiceAction::Restart, "Raspotify".into())
    );
    assert_eq!(
        control.performed,
        [("Raspotify".to_owned(), ServiceAction::Restart)]
    );
    for (result, expected) in [
        (Err(ServiceControlError::Unknown), RemoteError::Conflict),
        (Err(ServiceControlError::NotAllowed), RemoteError::Conflict),
        (Err(ServiceControlError::Busy), RemoteError::Busy),
        (
            Err(ServiceControlError::Unavailable),
            RemoteError::Unavailable,
        ),
    ] {
        control.result = result;
        assert_eq!(
            call(
                &mut client,
                &mut device,
                &mut store,
                &mut control,
                Operation::ServiceAction,
                &payload
            ),
            ResponseBody::Error(expected)
        );
    }
    control.performed.clear();
    // Malformed payloads never reach the control.
    assert!(
        client
            .request_payload(Operation::ServiceAction, &[9, 1, b'x'], NOW)
            .is_err()
            || control.performed.is_empty()
    );
    control.supported = false;
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut control,
            Operation::Capabilities,
            &[]
        ),
        ResponseBody::Capabilities(CAPABILITIES)
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut control,
            Operation::ServiceAction,
            &payload
        ),
        ResponseBody::Error(RemoteError::Unsupported)
    );
}

fn rpc_payload(
    client: &mut ClientSession,
    device: &mut DeviceSession,
    store: &mut impl StateStore,
    operation: Operation,
    payload: &[u8],
) -> ResponseBody {
    let request = client.request_payload(operation, payload, NOW).unwrap();
    let response = device
        .handle(&request, store, &mut SimulatedNetwork::default(), time(NOW))
        .unwrap();
    client.receive(&response, NOW).unwrap().body
}
fn authorized_pair(
    client_identity: &IdentityHandle,
    device_identity: &IdentityHandle,
    sessions: &SessionRegistry,
    store: &mut impl StateStore,
    over_ip: bool,
) -> (ClientSession, DeviceSession) {
    let (client, device) = raw_pair(client_identity, device_identity);
    let mut client = ClientSession::new(client).unwrap();
    let mut device = if over_ip {
        DeviceSession::new_ip(device, sessions)
    } else {
        DeviceSession::new(device, sessions)
    }
    .unwrap();
    assert_eq!(
        rpc(&mut client, &mut device, store, Operation::Register),
        ResponseBody::Registered
    );
    (client, device)
}

#[test]
fn a_correlation_proof_ties_an_ip_session_to_a_running_session_exactly_once() {
    let (client_identity, device_identity) = (identity(), identity());
    let sessions = SessionRegistry::default();
    let mut store = state();
    let (mut ble, mut ble_device) = authorized_pair(
        &client_identity,
        &device_identity,
        &sessions,
        &mut store,
        false,
    );
    let (mut ip, mut ip_device) = authorized_pair(
        &client_identity,
        &device_identity,
        &sessions,
        &mut store,
        true,
    );
    let ResponseBody::Correlation(token) = rpc(
        &mut ble,
        &mut ble_device,
        &mut store,
        Operation::CorrelationIssue,
    ) else {
        panic!("no proof issued");
    };
    let mut wrong = token.clone();
    wrong.0[0] ^= 1;
    for (proof, expected) in [
        (&wrong, ResponseBody::Error(RemoteError::Conflict)),
        (&token, ResponseBody::Correlated),
        (&token, ResponseBody::Error(RemoteError::Conflict)),
    ] {
        assert_eq!(
            rpc_payload(
                &mut ip,
                &mut ip_device,
                &mut store,
                Operation::CorrelationRedeem,
                &proof.0
            ),
            expected
        );
    }
}

#[test]
fn a_correlation_proof_is_only_redeemable_over_the_ip_endpoint_and_expires() {
    let (client_identity, device_identity) = (identity(), identity());
    let sessions = SessionRegistry::default();
    let mut store = state();
    let (mut ble, mut ble_device) = authorized_pair(
        &client_identity,
        &device_identity,
        &sessions,
        &mut store,
        false,
    );
    let ResponseBody::Correlation(token) = rpc(
        &mut ble,
        &mut ble_device,
        &mut store,
        Operation::CorrelationIssue,
    ) else {
        panic!("no proof issued");
    };
    // The same Bluetooth session cannot redeem its own proof.
    assert_eq!(
        rpc_payload(
            &mut ble,
            &mut ble_device,
            &mut store,
            Operation::CorrelationRedeem,
            &token.0
        ),
        ResponseBody::Error(RemoteError::Conflict)
    );
    // After the validity window the proof is gone.
    let (mut ip, mut ip_device) = authorized_pair(
        &client_identity,
        &device_identity,
        &sessions,
        &mut store,
        true,
    );
    let request = ip
        .request_payload(Operation::CorrelationRedeem, &token.0, NOW)
        .unwrap();
    let late = Duration::from_secs(31);
    let response = ip_device
        .handle(
            &request,
            &mut store,
            &mut SimulatedNetwork::default(),
            time(late),
        )
        .unwrap();
    assert_eq!(
        ip.receive(&response, late).unwrap().body,
        ResponseBody::Error(RemoteError::Conflict)
    );
}

#[test]
fn restart_option_is_encrypted_authorized_and_offered_only_when_supported() {
    use zorvinkel_application::{device::NetworkStatusSource, wifi::*};
    use zorvinkel_protocol::restart as wire;
    #[derive(Default)]
    struct Wifi {
        supported: bool,
        state: Option<RestartState>,
        changes: Vec<(u64, RestartChange)>,
    }
    impl NetworkStatusSource for Wifi {
        fn read_status(&mut self) -> Result<NetworkStatus, zorvinkel_application::PortError> {
            Ok(NetworkStatus::Disconnected)
        }
        fn wifi_setup(&mut self) -> Option<&mut dyn WifiSetupControl> {
            Some(self)
        }
    }
    impl WifiSetupControl for Wifi {
        fn scan(&mut self, _: &ClientKey, _: u64) -> Result<(), WifiError> {
            Err(WifiError::Unsupported)
        }
        fn connect(&mut self, _: &ClientKey, _: u64, _: &str, _: &str) -> Result<(), WifiError> {
            Err(WifiError::Unsupported)
        }
        fn poll(&mut self, _: &ClientKey, _: u64) -> Result<WifiTaskState, WifiError> {
            Ok(WifiTaskState::Connected)
        }
        fn restart_supported(&self) -> bool {
            self.supported
        }
        fn restart_state(&mut self, _: &ClientKey) -> Result<RestartState, WifiError> {
            self.state.ok_or(WifiError::Unavailable)
        }
        fn change_restart(
            &mut self,
            _: &ClientKey,
            id: u64,
            change: RestartChange,
        ) -> Result<(), WifiError> {
            if self.changes.len() >= 2 {
                return Err(WifiError::Busy);
            }
            self.changes.push((id, change));
            Ok(())
        }
    }
    fn call(
        client: &mut ClientSession,
        device: &mut DeviceSession,
        store: &mut MemoryStateStore,
        wifi: &mut Wifi,
        op: Operation,
        payload: &[u8],
    ) -> ResponseBody {
        let record = client.request_payload(op, payload, NOW).unwrap();
        let reply = device.handle(&record, store, wifi, time(NOW)).unwrap();
        client.receive(&reply, NOW).unwrap().body
    }
    let client_key = identity();
    let device_key = identity();
    let sessions = SessionRegistry::default();
    let pair = || {
        connect(
            &client_key,
            &device_key,
            &mut MemoryDeviceTrustStore::default(),
            TrustMode::TrustOnFirstUse,
            ChannelContext::Simulation,
            NOW,
            &sessions,
        )
        .unwrap()
    };
    let mut store = state();
    let mut wifi = Wifi {
        supported: true,
        state: Some(RestartState::DefaultWifi),
        changes: vec![],
    };
    let set = wire::set_payload(7, wire::RestartChange::CurrentAsDefault).unwrap();
    let (mut client, mut device) = pair();
    // Nothing before registration.
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::RestartPolicyRead,
            &[]
        ),
        ResponseBody::Error(RemoteError::Unauthorized)
    );
    let (mut client, mut device) = pair();
    call(
        &mut client,
        &mut device,
        &mut store,
        &mut wifi,
        Operation::Register,
        &[],
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::Capabilities,
            &[]
        ),
        ResponseBody::Capabilities(
            CAPABILITIES | zorvinkel_protocol::wifi::CAP_WIFI_TEMPORARY | wire::CAP_RESTART_POLICY
        )
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::RestartPolicyRead,
            &[]
        ),
        ResponseBody::RestartPolicy(wire::RestartState::DefaultWifi)
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::RestartPolicySet,
            &set
        ),
        ResponseBody::RestartPolicyStarted(7)
    );
    assert_eq!(wifi.changes, [(7, RestartChange::CurrentAsDefault)]);
    wifi.state = Some(RestartState::DefaultAccessPoint);
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::RestartPolicyRead,
            &[]
        ),
        ResponseBody::RestartPolicy(wire::RestartState::DefaultAccessPoint)
    );
    wifi.state = None;
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::RestartPolicyRead,
            &[]
        ),
        ResponseBody::Error(RemoteError::Unavailable)
    );
    let last = wire::set_payload(8, wire::RestartChange::LastAccepted).unwrap();
    call(
        &mut client,
        &mut device,
        &mut store,
        &mut wifi,
        Operation::RestartPolicySet,
        &last,
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::RestartPolicySet,
            &last
        ),
        ResponseBody::Error(RemoteError::Busy)
    );
    // A device without the option neither advertises nor serves it.
    wifi.supported = false;
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::Capabilities,
            &[]
        ),
        ResponseBody::Capabilities(CAPABILITIES | zorvinkel_protocol::wifi::CAP_WIFI_TEMPORARY)
    );
    assert_eq!(
        call(
            &mut client,
            &mut device,
            &mut store,
            &mut wifi,
            Operation::RestartPolicyRead,
            &[]
        ),
        ResponseBody::Error(RemoteError::Unsupported)
    );
}
