#![cfg(all(feature = "security", unix))]
#[allow(dead_code)]
mod support;
use std::time::Duration;
use support::Fixture;
use zorvinkel_adapters::{
    ble::ProbeSession,
    identity::IdentityHandle,
    persistence::{FileStateStore, catalog},
    security::{
        ChannelContext,
        ble::Responder,
        handshake::{Handshake, Role},
        identity::generate_identity,
        service::{ClientSession, ServiceTime, SessionRegistry},
        storage::{initialize_device, load_identity},
    },
    simulation::SimulatedNetwork,
};
use zorvinkel_application::state::StateStore;
use zorvinkel_core::registry::RegistrationState;
use zorvinkel_protocol::status::{Operation, ResponseBody};

fn framed(sender: &mut ProbeSession, receiver: &mut ProbeSession, record: &[u8]) -> Vec<u8> {
    let mut result = None;
    for frame in sender.send(record).unwrap() {
        result = receiver.receive(&frame, Duration::ZERO).unwrap();
    }
    result.unwrap()
}
fn connect(
    identity: &IdentityHandle,
    device: &IdentityHandle,
    store: &mut FileStateStore,
    sessions: &SessionRegistry,
) -> (ClientSession, Responder, ProbeSession, ProbeSession) {
    let mut client =
        Handshake::new(identity, Role::Client, ChannelContext::Ble, Duration::ZERO).unwrap();
    let mut responder = Responder::new(device, Duration::ZERO).unwrap();
    let (mut c, mut d) = (
        ProbeSession::with_kind(zorvinkel_adapters::ble::STATUS.kind),
        ProbeSession::with_kind(zorvinkel_adapters::ble::STATUS.kind),
    );
    let first = framed(&mut c, &mut d, &client.write(Duration::ZERO).unwrap());
    let second = responder
        .receive(
            &first,
            store,
            &mut SimulatedNetwork::default(),
            time(),
            sessions,
        )
        .unwrap()
        .unwrap();
    client
        .read(&framed(&mut d, &mut c, &second), Duration::ZERO)
        .unwrap();
    let third = framed(&mut c, &mut d, &client.write(Duration::ZERO).unwrap());
    assert!(
        responder
            .receive(
                &third,
                store,
                &mut SimulatedNetwork::default(),
                time(),
                sessions
            )
            .unwrap()
            .is_none()
    );
    (
        ClientSession::new(client.finish(Duration::ZERO).unwrap()).unwrap(),
        responder,
        c,
        d,
    )
}
fn time() -> ServiceTime {
    ServiceTime {
        elapsed: Duration::ZERO,
        registration_seconds: 100,
    }
}

#[test]
fn fragmented_ble_registration_survives_restart_and_lost_response() {
    let fixture = Fixture::new();
    let root = fixture.0.join("device");
    initialize_device(&root).unwrap();
    let identity = IdentityHandle::from_material(generate_identity().unwrap());
    let sessions = SessionRegistry::default();
    let mut store = FileStateStore::open(&root.join("registry")).unwrap();
    let device = load_identity(&root).unwrap();
    let key = device.public_key().to_vec();
    let (mut client, mut responder, mut c, mut d) =
        connect(&identity, &device, &mut store, &sessions);
    let request = framed(
        &mut c,
        &mut d,
        &client.request(Operation::Register, Duration::ZERO).unwrap(),
    );
    let _lost = responder
        .receive(
            &request,
            &mut store,
            &mut SimulatedNetwork::default(),
            time(),
            &sessions,
        )
        .unwrap();
    drop((client, responder, store, device));
    assert!(initialize_device(&root).is_err());
    let device = load_identity(&root).unwrap();
    assert_eq!(device.public_key(), key);
    let mut store = FileStateStore::open(&root.join("registry")).unwrap();
    assert_eq!(
        store.read().unwrap().registry.state,
        RegistrationState::Closed
    );
    let (mut client, mut responder, mut c, mut d) =
        connect(&identity, &device, &mut store, &sessions);
    for (operation, expected) in [
        (Operation::Authenticate, ResponseBody::Authenticated),
        (
            Operation::Status,
            ResponseBody::Status {
                mode: 0,
                ssid: String::new(),
            },
        ),
    ] {
        let request = framed(
            &mut c,
            &mut d,
            &client.request(operation, Duration::ZERO).unwrap(),
        );
        let response = responder
            .receive(
                &request,
                &mut store,
                &mut SimulatedNetwork::default(),
                time(),
                &sessions,
            )
            .unwrap()
            .unwrap();
        assert_eq!(
            client
                .receive(&framed(&mut d, &mut c, &response), Duration::ZERO)
                .unwrap()
                .body,
            expected
        );
    }
}
#[test]
fn missing_saved_binding_is_not_replaced_by_tofu() {
    let fixture = Fixture::new();
    catalog::initialize(&fixture.0).unwrap();
    let _ = catalog::select(&fixture.0, "device").unwrap();
    let path = fixture.0.join("device-646576696365");
    std::fs::remove_dir_all(path).unwrap();
    assert!(catalog::select(&fixture.0, "device").is_err());
}
#[test]
fn native_responder_expires_before_authentication_and_rejects_probe_data() {
    let fixture = Fixture::new();
    let root = fixture.0.join("device");
    initialize_device(&root).unwrap();
    let identity = load_identity(&root).unwrap();
    let mut responder = Responder::new(&identity, Duration::ZERO).unwrap();
    assert!(!responder.closed(Duration::from_secs(9)));
    assert!(responder.closed(Duration::from_secs(10)));
    let mut store = FileStateStore::open(&root.join("registry")).unwrap();
    assert!(
        responder
            .receive(
                &zorvinkel_protocol::probe::REQUEST,
                &mut store,
                &mut SimulatedNetwork::default(),
                time(),
                &SessionRegistry::default()
            )
            .is_err()
    );
    assert!(responder.closed(Duration::ZERO));
    assert_eq!(
        store.read().unwrap().registry.state,
        RegistrationState::Unclaimed
    );
}

#[cfg(all(target_os = "linux", feature = "ble-server"))]
#[test]
fn local_admin_grants_only_target_and_revokes_registered_client() {
    use zorvinkel_adapters::linux::{
        admin::{encode_key, execute},
        runtime::RuntimeState,
    };
    use zorvinkel_application::registration::register;
    use zorvinkel_core::registry::ClientKey;
    let fixture = Fixture::new();
    let root = fixture.0.join("device");
    initialize_device(&root).unwrap();
    let mut state = RuntimeState::open(&root).unwrap();
    let first_identity = IdentityHandle::from_material(generate_identity().unwrap());
    let first = ClientKey::new(first_identity.public_key().to_vec()).unwrap();
    let next = ClientKey::new(vec![2; 32]).unwrap();
    register(&mut state.store, first.clone(), 0).unwrap();
    execute(
        &mut state,
        &format!("grant {}", encode_key(next.as_bytes())),
    )
    .unwrap();
    let now = state.registration_seconds();
    assert!(register(&mut state.store, ClientKey::new(vec![3; 32]).unwrap(), now).is_err());
    register(&mut state.store, next.clone(), now).unwrap();
    let sessions = state.sessions.clone();
    let device = load_identity(&root).unwrap();
    let (mut client, mut responder, mut c, mut d) =
        connect(&first_identity, &device, &mut state.store, &sessions);
    let request = framed(
        &mut c,
        &mut d,
        &client
            .request(Operation::Authenticate, Duration::ZERO)
            .unwrap(),
    );
    responder
        .receive(
            &request,
            &mut state.store,
            &mut SimulatedNetwork::default(),
            time(),
            &sessions,
        )
        .unwrap();
    execute(
        &mut state,
        &format!("revoke {}", encode_key(first.as_bytes())),
    )
    .unwrap();
    assert!(responder.closed(Duration::ZERO));
    assert!(
        state
            .store
            .read()
            .unwrap()
            .registry
            .authorize(&first)
            .is_err()
    );
    assert!(execute(&mut state, "authorize malformed").is_err());
}

#[cfg(all(target_os = "linux", feature = "ble-server"))]
#[test]
fn read_only_status_never_treats_unavailable_observation_as_disconnected() {
    use zorvinkel_adapters::linux::status::parse_status;
    use zorvinkel_core::NetworkStatus;
    assert!(parse_status("FAIL\n").is_err());
    assert!(parse_status("wpa_state=COMPLETED\n").is_err());
    assert_eq!(
        parse_status("wpa_state=DISCONNECTED\n").unwrap(),
        NetworkStatus::Disconnected
    );
    assert_eq!(
        parse_status("ssid=Studio\nwpa_state=COMPLETED\nmode=station\n").unwrap(),
        NetworkStatus::WifiConnected {
            ssid: "Studio".into()
        }
    );
}

#[test]
fn registered_display_names_survive_reopening_without_changing_trust() {
    use zorvinkel_adapters::persistence::names;
    use zorvinkel_application::{
        device_names::{RegisteredName, apply},
        trust::DeviceTrustStore,
    };
    use zorvinkel_core::{DiscoveredDevice, trust::DeviceKey};
    let fixture = Fixture::new();
    let root = fixture.0.join("client");
    std::fs::create_dir(&root).unwrap();
    catalog::initialize(&root).unwrap();
    let mut trust = catalog::select(&root, "first").unwrap();
    let key = DeviceKey::new(vec![1; 32]).unwrap();
    trust
        .update(|binding| {
            binding.configure(key.clone()).unwrap();
            Ok(())
        })
        .unwrap();
    names::remember(
        &root,
        RegisteredName {
            handle: "first".into(),
            identity: [1; 32],
            hostname: "Pi".into(),
        },
    )
    .unwrap();
    names::remember(
        &root,
        RegisteredName {
            handle: "offline".into(),
            identity: [2; 32],
            hostname: "Pi".into(),
        },
    )
    .unwrap();
    let mut devices = vec![DiscoveredDevice {
        handle: "first".into(),
        display_name: "Pi-967C".into(),
    }];
    apply(&mut devices, &names::read(&root).unwrap());
    assert_eq!(devices[0].display_name, "Pi · 01010101");
    let path = root.join("device-names/state");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[0] ^= 1;
    std::fs::write(path, bytes).unwrap();
    assert!(names::read(&root).is_err());
    assert_eq!(
        trust
            .update(|binding| Ok(binding.expected_key().cloned()))
            .unwrap(),
        Some(key)
    );
}
