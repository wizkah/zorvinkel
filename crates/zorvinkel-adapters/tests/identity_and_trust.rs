#![cfg(unix)]
mod support;
use std::{fs, os::unix::fs::PermissionsExt};
use support::Fixture;
use zorvinkel_adapters::persistence::{
    identity::{FileIdentityStore, IdentityMaterial},
    trust::FileDeviceTrustStore,
};
use zorvinkel_application::{
    state::{ApplicationError, IdentityStore, StateError},
    trust::*,
};
use zorvinkel_core::trust::*;
fn key(value: u8) -> DeviceKey {
    DeviceKey::new(vec![value; 32]).unwrap()
}
fn material(value: u8) -> IdentityMaterial {
    IdentityMaterial::new(vec![value; 32], vec![value + 1; 32]).unwrap()
}

#[test]
fn identity_survives_reopen_without_replacement_or_private_export() {
    let fixture = Fixture::new();
    let mut store = FileIdentityStore::initialize(&fixture.directory(), material(1)).unwrap();
    let handle = store.load_identity().unwrap();
    assert_eq!(store.public_identity(&handle).unwrap(), vec![1; 32]);
    drop(store);
    assert!(FileIdentityStore::initialize(&fixture.directory(), material(3)).is_err());
    let mut reopened = FileIdentityStore::open(&fixture.directory()).unwrap();
    let handle = reopened.load_identity().unwrap();
    assert_eq!(reopened.public_identity(&handle).unwrap(), vec![1; 32]);
    assert_eq!(
        fs::metadata(fixture.directory())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(fixture.directory().join("state"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn lost_or_corrupt_identity_does_not_generate_a_new_identity() {
    let fixture = Fixture::new();
    let mut store = FileIdentityStore::initialize(&fixture.directory(), material(1)).unwrap();
    fs::write(fixture.directory().join("state"), b"broken").unwrap();
    assert!(matches!(store.load_identity(), Err(StateError::Corrupt)));
    assert!(matches!(
        FileIdentityStore::open(&fixture.directory()),
        Err(StateError::Corrupt)
    ));
    fs::remove_file(fixture.directory().join("state")).unwrap();
    assert!(matches!(store.load_identity(), Err(StateError::Missing)));
    assert!(FileIdentityStore::initialize(&fixture.directory(), material(2)).is_err());
}

#[test]
fn identity_rejects_insecure_permissions() {
    let fixture = Fixture::new();
    let mut store = FileIdentityStore::initialize(&fixture.directory(), material(1)).unwrap();
    fs::set_permissions(
        fixture.directory().join("state"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(matches!(store.load_identity(), Err(StateError::Io)));
}

#[test]
fn setup_device_key_is_persistent_and_cannot_be_silently_replaced() {
    let fixture = Fixture::new();
    let mut store = FileDeviceTrustStore::initialize(&fixture.directory()).unwrap();
    configure_device(&mut store, key(1)).unwrap();
    let mut reopened = FileDeviceTrustStore::open(&fixture.directory()).unwrap();
    verify_device(&mut reopened, key(1), TrustMode::RequireKnownKey).unwrap();
    assert_eq!(
        verify_device(&mut reopened, key(2), TrustMode::TrustOnFirstUse),
        Err(TrustError::KeyMismatch.into())
    );
    assert_eq!(
        configure_device(&mut reopened, key(2)),
        Err(TrustError::KeyMismatch.into())
    );
    verify_device(&mut reopened, key(1), TrustMode::RequireKnownKey).unwrap();
}

#[test]
fn tofu_binding_commit_failure_is_not_reported_as_trusted() {
    let fixture = Fixture::new();
    let mut store = FileDeviceTrustStore::initialize(&fixture.directory()).unwrap();
    fs::create_dir(fixture.directory().join("next")).unwrap();
    assert_eq!(
        verify_device(&mut store, key(1), TrustMode::TrustOnFirstUse),
        Err(ApplicationError::Storage(StateError::Io))
    );
    fs::remove_dir(fixture.directory().join("next")).unwrap();
    // The failed commit did not bind the old key.
    verify_device(&mut store, key(2), TrustMode::TrustOnFirstUse).unwrap();
    let mut reopened = FileDeviceTrustStore::open(&fixture.directory()).unwrap();
    assert_eq!(
        verify_device(&mut reopened, key(1), TrustMode::TrustOnFirstUse),
        Err(TrustError::KeyMismatch.into())
    );
}

#[test]
fn missing_or_corrupt_trust_record_never_falls_back_to_tofu() {
    let fixture = Fixture::new();
    let mut store = FileDeviceTrustStore::initialize(&fixture.directory()).unwrap();
    configure_device(&mut store, key(1)).unwrap();
    fs::write(fixture.directory().join("state"), b"broken").unwrap();
    assert_eq!(
        verify_device(&mut store, key(2), TrustMode::TrustOnFirstUse),
        Err(ApplicationError::Storage(StateError::Corrupt))
    );
    fs::remove_file(fixture.directory().join("state")).unwrap();
    assert!(matches!(
        FileDeviceTrustStore::open(&fixture.directory()),
        Err(StateError::Missing)
    ));
    assert!(FileDeviceTrustStore::initialize(&fixture.directory()).is_err());
}

#[test]
fn setup_authorized_client_is_preserved_after_restart() {
    use zorvinkel_application::{registration::*, state::StateStore};
    use zorvinkel_core::registry::{ClientKey, RegistrationState};
    let fixture = Fixture::new();
    let mut store = fixture.initialize();
    let client = ClientKey::new(vec![1; 32]).unwrap();
    authorize_by_local_admin(&mut store, client.clone()).unwrap();
    let mut reopened =
        zorvinkel_adapters::persistence::FileStateStore::open(&fixture.directory()).unwrap();
    register(&mut reopened, client, 0).unwrap();
    assert_eq!(
        reopened.read().unwrap().registry.state,
        RegistrationState::Closed
    );
    assert!(register(&mut reopened, ClientKey::new(vec![2; 32]).unwrap(), 0).is_err());
}

#[test]
fn independent_trust_handles_cannot_bind_different_first_keys() {
    let fixture = Fixture::new();
    FileDeviceTrustStore::initialize(&fixture.directory()).unwrap();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = (1..=2)
        .map(|value| {
            let mut store = FileDeviceTrustStore::open(&fixture.directory()).unwrap();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                verify_device(&mut store, key(value), TrustMode::TrustOnFirstUse)
            })
        })
        .collect();
    assert_eq!(
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .filter(Result::is_ok)
            .count(),
        1
    );
}

#[test]
fn failed_setup_authorization_does_not_partially_register_the_key() {
    use zorvinkel_application::{registration::authorize_by_local_admin, state::StateStore};
    use zorvinkel_core::registry::{ClientKey, RegistrationState};
    let fixture = Fixture::new();
    let mut store = fixture.initialize();
    fs::create_dir(fixture.directory().join("next")).unwrap();
    assert_eq!(
        authorize_by_local_admin(&mut store, ClientKey::new(vec![1; 32]).unwrap()),
        Err(ApplicationError::Storage(StateError::Io))
    );
    let state = store.read().unwrap();
    assert!(state.registry.clients.is_empty());
    assert_eq!(state.registry.state, RegistrationState::Unclaimed);
}
