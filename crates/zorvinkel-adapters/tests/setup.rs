use zorvinkel_adapters::memory::{MemoryDeviceTrustStore, MemoryStateStore};
use zorvinkel_application::{registration::*, state::*, trust::*};
use zorvinkel_core::{network::NetworkConfiguration, registry::*, trust::*};
fn key(value: u8) -> ClientKey {
    ClientKey::new(vec![value; 32]).unwrap()
}
fn device_key(value: u8) -> DeviceKey {
    DeviceKey::new(vec![value; 32]).unwrap()
}
fn store() -> MemoryStateStore {
    MemoryStateStore::new(DeviceState::initial(NetworkConfiguration::Disconnected))
}

#[test]
fn setup_authorizes_client_and_closes_first_registration_atomically() {
    let mut store = store();
    authorize_by_local_admin(&mut store, key(1)).unwrap();
    let state = store.read().unwrap();
    assert_eq!(state.registry.state, RegistrationState::Closed);
    assert!(state.registry.authorize(&key(1)).is_ok());
    assert_eq!(
        register(&mut store, key(2), 0),
        Err(RegistryError::Closed.into())
    );
    register(&mut store, key(1), 0).unwrap();
}

#[test]
fn setup_can_add_keys_after_tofu_without_reopening_registration() {
    let mut store = store();
    register(&mut store, key(1), 0).unwrap();
    authorize_by_client(&mut store, &key(1), key(2)).unwrap();
    authorize_by_client(&mut store, &key(1), key(2)).unwrap();
    assert_eq!(store.read().unwrap().registry.clients.len(), 2);
    assert_eq!(
        store.read().unwrap().registry.state,
        RegistrationState::Closed
    );
    assert!(register(&mut store, key(3), 0).is_err());
}

#[test]
fn unknown_and_revoked_keys_cannot_bypass_setup_authorization() {
    let mut store = store();
    let before = store.read().unwrap();
    assert_eq!(
        authorize_by_client(&mut store, &key(9), key(1)),
        Err(RegistryError::Unauthorized.into())
    );
    assert_eq!(store.read().unwrap(), before);
    register(&mut store, key(1), 0).unwrap();
    authorize_by_client(&mut store, &key(1), key(2)).unwrap();
    revoke(&mut store, &key(1), &key(2)).unwrap();
    assert_eq!(
        authorize_by_client(&mut store, &key(2), key(3)),
        Err(RegistryError::Unauthorized.into())
    );
    assert_eq!(
        authorize_by_local_admin(&mut store, key(2)),
        Err(RegistryError::Revoked.into())
    );
}

#[test]
fn client_binds_first_key_and_rejects_changed_key() {
    let mut store = MemoryDeviceTrustStore::default();
    verify_device(&mut store, device_key(1), TrustMode::TrustOnFirstUse).unwrap();
    verify_device(&mut store, device_key(1), TrustMode::TrustOnFirstUse).unwrap();
    assert_eq!(
        verify_device(&mut store, device_key(2), TrustMode::TrustOnFirstUse),
        Err(TrustError::KeyMismatch.into())
    );
    verify_device(&mut store, device_key(1), TrustMode::RequireKnownKey).unwrap();
}

#[test]
fn setup_pin_is_checked_on_first_connection_without_tofu_fallback() {
    let mut store = MemoryDeviceTrustStore::default();
    assert_eq!(
        verify_device(&mut store, device_key(1), TrustMode::RequireKnownKey),
        Err(TrustError::UnknownDevice.into())
    );
    configure_device(&mut store, device_key(2)).unwrap();
    assert_eq!(
        verify_device(&mut store, device_key(1), TrustMode::TrustOnFirstUse),
        Err(TrustError::KeyMismatch.into())
    );
    assert_eq!(
        configure_device(&mut store, device_key(1)),
        Err(TrustError::KeyMismatch.into())
    );
    verify_device(&mut store, device_key(2), TrustMode::RequireKnownKey).unwrap();
}

#[test]
fn concurrent_first_device_bindings_have_one_winner() {
    let store = MemoryDeviceTrustStore::default();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = (1..=2)
        .map(|value| {
            let mut store = store.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                verify_device(&mut store, device_key(value), TrustMode::TrustOnFirstUse)
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
