#![cfg(unix)]
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::{Arc, Barrier},
};
use zorvinkel_adapters::persistence::FileStateStore;
use zorvinkel_application::{registration::*, state::*};
use zorvinkel_core::{network::*, registry::*};

mod support;
use support::Fixture;

fn key(value: u8) -> ClientKey {
    ClientKey::new(vec![value; 32]).unwrap()
}

#[test]
fn registration_commit_survives_reopen_and_duplicate_response() {
    let fixture = Fixture::new();
    let mut store = fixture.initialize();
    register(&mut store, key(1), 0).unwrap();
    drop(store);
    let mut reopened = FileStateStore::open(&fixture.directory()).unwrap();
    register(&mut reopened, key(1), 0).unwrap();
    assert!(register(&mut reopened, key(2), 0).is_err());
    assert_eq!(
        reopened.read().unwrap().registry.state,
        RegistrationState::Closed
    );
    assert_eq!(
        fs::metadata(fixture.directory())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    for file in ["state", "lock"] {
        assert_eq!(
            fs::metadata(fixture.directory().join(file))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    assert!(
        FileStateStore::initialize(
            &fixture.directory(),
            DeviceState::initial(NetworkConfiguration::Disconnected)
        )
        .is_err()
    );
}

#[test]
fn independent_store_handles_serialize_first_registration() {
    let fixture = Fixture::new();
    fixture.initialize();
    let barrier = Arc::new(Barrier::new(2));
    let workers: Vec<_> = (1..=2)
        .map(|value| {
            let mut store = FileStateStore::open(&fixture.directory()).unwrap();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                register(&mut store, key(value), 0)
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
    let state = FileStateStore::open(&fixture.directory())
        .unwrap()
        .read()
        .unwrap();
    assert_eq!(state.registry.clients.len(), 1);
    assert_eq!(state.registry.state, RegistrationState::Closed);
}

#[test]
fn missing_or_damaged_state_never_reinitializes() {
    let fixture = Fixture::new();
    let mut store = fixture.initialize();
    register(&mut store, key(1), 0).unwrap();
    fs::write(fixture.directory().join("state"), b"broken").unwrap();
    assert!(matches!(
        FileStateStore::open(&fixture.directory()),
        Err(StateError::Corrupt)
    ));
    assert!(matches!(
        register(&mut store, key(2), 0),
        Err(ApplicationError::Storage(StateError::Corrupt))
    ));
    fs::remove_file(fixture.directory().join("state")).unwrap();
    assert!(matches!(
        FileStateStore::open(&fixture.directory()),
        Err(StateError::Missing)
    ));
    assert!(
        FileStateStore::initialize(
            &fixture.directory(),
            DeviceState::initial(NetworkConfiguration::Disconnected)
        )
        .is_err()
    );
}

#[test]
fn incomplete_temporary_snapshot_is_ignored_after_restart() {
    let fixture = Fixture::new();
    let mut store = fixture.initialize();
    register(&mut store, key(1), 0).unwrap();
    fs::write(fixture.directory().join("next"), b"partial write").unwrap();
    let mut reopened = FileStateStore::open(&fixture.directory()).unwrap();
    assert_eq!(
        reopened.read().unwrap().registry.state,
        RegistrationState::Closed
    );
    grant_by_local_admin(&mut reopened, key(2), 0, 10).unwrap();
    register(&mut reopened, key(2), 1).unwrap();
    assert_eq!(reopened.read().unwrap().registry.clients.len(), 2);
}

#[test]
fn failed_commit_does_not_report_registration_success() {
    let fixture = Fixture::new();
    let mut store = fixture.initialize();
    fs::create_dir(fixture.directory().join("next")).unwrap();
    assert_eq!(
        register(&mut store, key(1), 0),
        Err(ApplicationError::Storage(StateError::Io))
    );
    assert_eq!(
        store.read().unwrap().registry.state,
        RegistrationState::Unclaimed
    );
    fs::remove_dir(fixture.directory().join("next")).unwrap();
    register(&mut store, key(2), 0).unwrap();
}

#[test]
fn revoked_last_client_stays_closed_after_restart() {
    let fixture = Fixture::new();
    let mut store = fixture.initialize();
    register(&mut store, key(1), 0).unwrap();
    revoke(&mut store, &key(1), &key(1)).unwrap();
    let mut reopened = FileStateStore::open(&fixture.directory()).unwrap();
    assert!(register(&mut reopened, key(2), 0).is_err());
    assert_eq!(
        register(&mut reopened, key(1), 0),
        Err(ApplicationError::Registry(RegistryError::Revoked))
    );
    grant_by_local_admin(&mut reopened, key(2), 0, 10).unwrap();
    register(&mut reopened, key(2), 1).unwrap();
}

#[test]
fn insecure_permissions_and_symlinks_are_rejected() {
    let fixture = Fixture::new();
    fixture.initialize();
    let path = fixture.directory().join("state");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(matches!(
        FileStateStore::open(&fixture.directory()),
        Err(StateError::Io)
    ));
    fs::rename(&path, fixture.0.join("moved")).unwrap();
    std::os::unix::fs::symlink(fixture.0.join("moved"), path).unwrap();
    assert!(matches!(
        FileStateStore::open(&fixture.directory()),
        Err(StateError::Io)
    ));
}

#[test]
fn journal_and_restart_settings_survive_reopen() {
    use zorvinkel_adapters::simulation::SimulatedNetworkControl;
    use zorvinkel_application::network::*;
    let fixture = Fixture::new();
    let mut store = fixture.initialize();
    register(&mut store, key(1), 0).unwrap();
    let intent = NetworkIntent::EnableAccessPoint(ProfileId::new("ap".into()).unwrap());
    let mut network = SimulatedNetworkControl {
        configuration: NetworkConfiguration::Disconnected,
        apply_error: None,
        restore_error: None,
        apply_count: 0,
    };
    execute(
        &mut store,
        &mut network,
        &key(1),
        OperationId(1),
        intent.clone(),
    )
    .unwrap();
    set_restart_policy(
        &mut store,
        &key(1),
        RestartPolicy::DefaultProfile(NetworkConfiguration::Wifi(
            ProfileId::new("default".into()).unwrap(),
        )),
    )
    .unwrap();
    let before = store.read().unwrap();
    let mut reopened = FileStateStore::open(&fixture.directory()).unwrap();
    assert_eq!(reopened.read().unwrap(), before);
    execute(&mut reopened, &mut network, &key(1), OperationId(1), intent).unwrap();
    assert_eq!(network.apply_count, 1);
}

#[test]
fn interrupted_operation_survives_restart_without_becoming_accepted() {
    use zorvinkel_adapters::simulation::SimulatedNetworkControl;
    use zorvinkel_application::network::*;
    struct InterruptedNetwork;
    impl NetworkControl for InterruptedNetwork {
        fn check_support(&mut self, _: &NetworkIntent) -> Result<(), NetworkError> {
            Ok(())
        }
        fn observe(&mut self) -> Result<NetworkConfiguration, NetworkError> {
            Ok(NetworkConfiguration::Disconnected)
        }
        fn apply(&mut self, _: &NetworkIntent) -> Result<(), NetworkError> {
            panic!("simulated process interruption");
        }
        fn restore(&mut self, _: &NetworkConfiguration) -> Result<(), NetworkError> {
            unreachable!()
        }
    }
    let fixture = Fixture::new();
    let mut store = fixture.initialize();
    register(&mut store, key(1), 0).unwrap();
    let intent = NetworkIntent::ConnectWifi(ProfileId::new("wifi".into()).unwrap());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(
            &mut store,
            &mut InterruptedNetwork,
            &key(1),
            OperationId(1),
            intent
        )))
        .is_err()
    );
    let mut reopened = FileStateStore::open(&fixture.directory()).unwrap();
    assert_eq!(
        reopened.read().unwrap().operations[&OperationId(1)].status,
        OperationStatus::Pending
    );
    let mut network = SimulatedNetworkControl {
        configuration: NetworkConfiguration::Wifi(ProfileId::new("wifi".into()).unwrap()),
        apply_error: None,
        restore_error: None,
        apply_count: 0,
    };
    reconcile(&mut reopened, &mut network).unwrap();
    let recovered = FileStateStore::open(&fixture.directory())
        .unwrap()
        .read()
        .unwrap();
    assert!(matches!(
        recovered.operations[&OperationId(1)].status,
        OperationStatus::Failed {
            cause: NetworkError::Interrupted,
            ..
        }
    ));
    assert_eq!(
        recovered.settings.last_accepted,
        NetworkConfiguration::Disconnected
    );
    assert_eq!(network.apply_count, 0);
}
