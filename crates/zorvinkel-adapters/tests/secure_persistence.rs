#![cfg(all(feature = "security", unix))]
mod support;
use std::time::Duration;
use support::Fixture;
use zorvinkel_adapters::{
    persistence::{FileStateStore, identity::FileIdentityStore, trust::FileDeviceTrustStore},
    security::{
        ChannelContext, SecureError, identity::generate_identity, service::*, simulation::connect,
    },
    simulation::SimulatedNetwork,
};
use zorvinkel_application::state::{IdentityStore, StateStore};
use zorvinkel_core::{registry::RegistrationState, trust::TrustMode};
use zorvinkel_protocol::status::*;

#[test]
fn generated_identities_registration_and_device_binding_survive_restart_and_lost_answer() {
    let fixture = Fixture::new();
    let client_path = fixture.0.join("client-identity");
    let device_path = fixture.0.join("device-identity");
    let trust_path = fixture.0.join("trust");
    let mut client_store =
        FileIdentityStore::initialize(&client_path, generate_identity().unwrap()).unwrap();
    let mut device_store =
        FileIdentityStore::initialize(&device_path, generate_identity().unwrap()).unwrap();
    let mut registry = fixture.initialize();
    let mut trust = FileDeviceTrustStore::initialize(&trust_path).unwrap();
    let client_identity = client_store.load_identity().unwrap();
    let device_identity = device_store.load_identity().unwrap();
    let client_public = client_identity.public_key().to_vec();
    let device_public = device_identity.public_key().to_vec();
    let sessions = SessionRegistry::default();
    let (mut client, mut device) = connect(
        &client_identity,
        &device_identity,
        &mut trust,
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        Duration::ZERO,
        &sessions,
    )
    .unwrap();
    let request = client.request(Operation::Register, Duration::ZERO).unwrap();
    let _lost_response = device
        .handle(
            &request,
            &mut registry,
            &mut SimulatedNetwork::default(),
            ServiceTime {
                elapsed: Duration::ZERO,
                registration_seconds: 0,
            },
        )
        .unwrap();
    drop((
        client,
        device,
        client_identity,
        device_identity,
        client_store,
        device_store,
        registry,
        trust,
    ));
    let client_identity = FileIdentityStore::open(&client_path)
        .unwrap()
        .load_identity()
        .unwrap();
    let device_identity = FileIdentityStore::open(&device_path)
        .unwrap()
        .load_identity()
        .unwrap();
    assert_eq!(client_identity.public_key(), client_public);
    assert_eq!(device_identity.public_key(), device_public);
    let mut registry = FileStateStore::open(&fixture.directory()).unwrap();
    let mut trust = FileDeviceTrustStore::open(&trust_path).unwrap();
    assert_eq!(
        registry.read().unwrap().registry.state,
        RegistrationState::Closed
    );
    let (mut client, mut device) = connect(
        &client_identity,
        &device_identity,
        &mut trust,
        TrustMode::RequireKnownKey,
        ChannelContext::Simulation,
        Duration::ZERO,
        &sessions,
    )
    .unwrap();
    for operation in [Operation::Authenticate, Operation::Status] {
        let request = client.request(operation, Duration::ZERO).unwrap();
        let response = device
            .handle(
                &request,
                &mut registry,
                &mut SimulatedNetwork::default(),
                ServiceTime {
                    elapsed: Duration::ZERO,
                    registration_seconds: 0,
                },
            )
            .unwrap();
        let body = client.receive(&response, Duration::ZERO).unwrap().body;
        assert!(matches!(
            body,
            ResponseBody::Authenticated | ResponseBody::Status { .. }
        ));
    }
    assert_eq!(registry.read().unwrap().registry.clients.len(), 1);
    assert_eq!(
        client.request(Operation::Authenticate, Duration::from_secs(600)),
        Err(SecureError::TimedOut)
    );
}
