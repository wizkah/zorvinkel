use zorvinkel_adapters::{memory::MemoryStateStore, simulation::SimulatedNetworkControl};
use zorvinkel_application::{network::*, registration::*, state::*};
use zorvinkel_core::{network::*, registry::*};

fn key(value: u8) -> ClientKey {
    ClientKey::new(vec![value; 32]).unwrap()
}
fn profile(value: &str) -> ProfileId {
    ProfileId::new(value.into()).unwrap()
}
fn store() -> MemoryStateStore {
    MemoryStateStore::new(DeviceState::initial(NetworkConfiguration::Disconnected))
}
fn network() -> SimulatedNetworkControl {
    SimulatedNetworkControl {
        configuration: NetworkConfiguration::Disconnected,
        apply_error: None,
        restore_error: None,
        apply_count: 0,
    }
}
fn claimed() -> MemoryStateStore {
    let mut store = store();
    register(&mut store, key(1), 0).unwrap();
    store
}

#[test]
fn first_registration_closes_and_lost_answer_is_idempotent() {
    let mut store = store();
    register(&mut store, key(1), 0).unwrap();
    assert_eq!(
        store.read().unwrap().registry.state,
        RegistrationState::Closed
    );
    register(&mut store, key(1), 1).unwrap();
    assert_eq!(
        register(&mut store, key(2), 1),
        Err(ApplicationError::Registry(RegistryError::Closed))
    );
    assert_eq!(store.read().unwrap().registry.clients.len(), 1);
}

#[test]
fn concurrent_first_contacts_have_one_winner() {
    let store = store();
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let workers: Vec<_> = (1..=2)
        .map(|value| {
            let mut store = store.clone();
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
}

#[test]
fn grants_are_targeted_one_time_and_expire_at_deadline() {
    let mut store = claimed();
    assert_eq!(
        grant_by_client(&mut store, &key(9), key(2), 1, 10),
        Err(ApplicationError::Registry(RegistryError::Unauthorized))
    );
    grant_by_client(&mut store, &key(1), key(2), 1, 10).unwrap();
    assert!(register(&mut store, key(3), 2).is_err());
    register(&mut store, key(2), 9).unwrap();
    assert_eq!(
        store.read().unwrap().registry.state,
        RegistrationState::Closed
    );
    grant_by_client(&mut store, &key(1), key(3), 10, 20).unwrap();
    assert!(register(&mut store, key(3), 20).is_err());
    assert_eq!(
        store.read().unwrap().registry.state,
        RegistrationState::Closed
    );
}

#[test]
fn losing_last_client_does_not_reopen_registration() {
    let mut store = claimed();
    revoke(&mut store, &key(1), &key(1)).unwrap();
    assert!(store.read().unwrap().registry.clients.is_empty());
    assert!(register(&mut store, key(2), 0).is_err());
    assert_eq!(
        register(&mut store, key(1), 0),
        Err(ApplicationError::Registry(RegistryError::Revoked))
    );
    grant_by_local_admin(&mut store, key(2), 0, 10).unwrap();
    register(&mut store, key(2), 1).unwrap();
    assert!(store.read().unwrap().registry.authorize(&key(2)).is_ok());
}

#[test]
fn successful_change_is_journaled_and_not_repeated() {
    let mut store = claimed();
    let mut network = network();
    let intent = NetworkIntent::ConnectWifi(profile("wifi"));
    let first = execute(
        &mut store,
        &mut network,
        &key(1),
        OperationId(1),
        intent.clone(),
    )
    .unwrap();
    assert!(matches!(first.status, OperationStatus::Succeeded { .. }));
    assert_eq!(
        execute(&mut store, &mut network, &key(1), OperationId(1), intent).unwrap(),
        first
    );
    assert_eq!(network.apply_count, 1);
    assert_eq!(
        store.read().unwrap().settings.last_accepted,
        NetworkConfiguration::Wifi(profile("wifi"))
    );
    assert_eq!(
        execute(
            &mut store,
            &mut network,
            &key(1),
            OperationId(1),
            NetworkIntent::DisableAccessPoint
        ),
        Err(ApplicationError::Conflict)
    );
}

#[test]
fn unauthorized_change_has_no_effect() {
    let mut store = claimed();
    let mut network = network();
    assert!(
        execute(
            &mut store,
            &mut network,
            &key(9),
            OperationId(1),
            NetworkIntent::DisableAccessPoint
        )
        .is_err()
    );
    assert_eq!(network.apply_count, 0);
    assert!(store.read().unwrap().operations.is_empty());
}

#[test]
fn failed_change_restores_previous_state_and_allows_correction() {
    let mut store = claimed();
    let mut network = network();
    network.apply_error = Some(NetworkError::TimedOut);
    let intent = NetworkIntent::EnableAccessPoint(profile("ap"));
    let failed = execute(
        &mut store,
        &mut network,
        &key(1),
        OperationId(1),
        intent.clone(),
    )
    .unwrap();
    assert_eq!(
        failed.status,
        OperationStatus::Failed {
            cause: NetworkError::TimedOut,
            actual: NetworkConfiguration::Disconnected,
            rollback_failed: false
        }
    );
    network.apply_error = None;
    assert!(matches!(
        execute(&mut store, &mut network, &key(1), OperationId(2), intent)
            .unwrap()
            .status,
        OperationStatus::Succeeded { .. }
    ));
}

#[test]
fn rollback_failure_reports_actual_state_without_accepting_it() {
    let mut store = claimed();
    let mut network = network();
    network.apply_error = Some(NetworkError::Rejected);
    network.restore_error = Some(NetworkError::Unavailable);
    let result = execute(
        &mut store,
        &mut network,
        &key(1),
        OperationId(1),
        NetworkIntent::EnableAccessPoint(profile("ap")),
    )
    .unwrap();
    assert!(matches!(
        result.status,
        OperationStatus::Failed {
            rollback_failed: true,
            actual: NetworkConfiguration::AccessPoint(_),
            ..
        }
    ));
    assert_eq!(
        store.read().unwrap().settings.last_accepted,
        NetworkConfiguration::Disconnected
    );
}

#[test]
fn pending_change_blocks_mutations_and_restart_reconciles_without_replay() {
    let mut store = claimed();
    let mut network = network();
    store
        .update(|state| {
            state.operations.insert(
                OperationId(1),
                NetworkOperation {
                    id: OperationId(1),
                    owner: key(1),
                    intent: NetworkIntent::ConnectWifi(profile("wifi")),
                    previous: NetworkConfiguration::Disconnected,
                    status: OperationStatus::Pending,
                },
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(
        execute(
            &mut store,
            &mut network,
            &key(1),
            OperationId(2),
            NetworkIntent::DisableAccessPoint
        ),
        Err(ApplicationError::Busy)
    );
    network.configuration = NetworkConfiguration::Wifi(profile("wifi"));
    reconcile(&mut store, &mut network).unwrap();
    let state = store.read().unwrap();
    assert!(matches!(
        state.operations[&OperationId(1)].status,
        OperationStatus::Failed {
            cause: NetworkError::Interrupted,
            ..
        }
    ));
    assert_eq!(
        state.settings.last_accepted,
        NetworkConfiguration::Disconnected
    );
    assert_eq!(network.apply_count, 0);
}

#[test]
fn both_restart_policies_support_wifi_and_access_point() {
    for configuration in [
        NetworkConfiguration::Wifi(profile("wifi")),
        NetworkConfiguration::AccessPoint(profile("ap")),
    ] {
        let mut store = claimed();
        store
            .update(|state| {
                state.settings.last_accepted = configuration.clone();
                Ok(())
            })
            .unwrap();
        assert_eq!(
            store.read().unwrap().settings.restart_configuration(),
            &configuration
        );
        set_restart_policy(
            &mut store,
            &key(1),
            RestartPolicy::DefaultProfile(configuration.clone()),
        )
        .unwrap();
        assert_eq!(
            store.read().unwrap().settings.restart_configuration(),
            &configuration
        );
    }
}

#[test]
fn aborted_transaction_keeps_previous_state() {
    let mut store = store();
    let before = store.read().unwrap();
    let result: Result<(), _> = store.update(|state| {
        state.registry.register(key(1), 0)?;
        Err(ApplicationError::Conflict)
    });
    assert_eq!(result, Err(ApplicationError::Conflict));
    assert_eq!(store.read().unwrap(), before);
}

struct FailingStore {
    inner: MemoryStateStore,
    remaining_commits: usize,
}
impl StateStore for FailingStore {
    fn read(&mut self) -> Result<DeviceState, StateError> {
        self.inner.read()
    }
    fn update<T>(
        &mut self,
        change: impl FnOnce(&mut DeviceState) -> Result<T, ApplicationError>,
    ) -> Result<T, ApplicationError> {
        if self.remaining_commits == 0 {
            return Err(StateError::Io.into());
        }
        self.remaining_commits -= 1;
        self.inner.update(change)
    }
}

#[test]
fn journal_commit_failure_prevents_system_change() {
    let mut store = FailingStore {
        inner: claimed(),
        remaining_commits: 0,
    };
    let mut network = network();
    assert_eq!(
        execute(
            &mut store,
            &mut network,
            &key(1),
            OperationId(1),
            NetworkIntent::EnableAccessPoint(profile("ap"))
        ),
        Err(ApplicationError::Storage(StateError::Io))
    );
    assert_eq!(network.apply_count, 0);
    assert!(store.read().unwrap().operations.is_empty());
}

#[test]
fn completion_commit_failure_requires_reconciliation_without_repeating_effect() {
    let mut store = FailingStore {
        inner: claimed(),
        remaining_commits: 1,
    };
    let mut network = network();
    let intent = NetworkIntent::EnableAccessPoint(profile("ap"));
    assert_eq!(
        execute(
            &mut store,
            &mut network,
            &key(1),
            OperationId(1),
            intent.clone()
        ),
        Err(ApplicationError::Storage(StateError::Io))
    );
    assert_eq!(
        execute(&mut store, &mut network, &key(1), OperationId(1), intent)
            .unwrap()
            .status,
        OperationStatus::Pending
    );
    assert_eq!(network.apply_count, 1);
    store.remaining_commits = 1;
    reconcile(&mut store, &mut network).unwrap();
    assert_eq!(
        store.read().unwrap().settings.last_accepted,
        NetworkConfiguration::Disconnected
    );
}

#[test]
fn disabling_access_point_preserves_an_existing_wifi_connection() {
    let mut store = claimed();
    let mut network = network();
    network.configuration = NetworkConfiguration::Wifi(profile("wifi"));
    execute(
        &mut store,
        &mut network,
        &key(1),
        OperationId(1),
        NetworkIntent::DisableAccessPoint,
    )
    .unwrap();
    assert_eq!(
        network.configuration,
        NetworkConfiguration::Wifi(profile("wifi"))
    );
}

#[test]
fn unknown_actual_state_keeps_operation_blocked_until_reconciliation() {
    struct LostNetwork {
        applied: bool,
    }
    impl NetworkControl for LostNetwork {
        fn check_support(&mut self, _: &NetworkIntent) -> Result<(), NetworkError> {
            Ok(())
        }
        fn observe(&mut self) -> Result<NetworkConfiguration, NetworkError> {
            if self.applied {
                Err(NetworkError::Unavailable)
            } else {
                Ok(NetworkConfiguration::Disconnected)
            }
        }
        fn apply(&mut self, _: &NetworkIntent) -> Result<(), NetworkError> {
            self.applied = true;
            Err(NetworkError::TimedOut)
        }
        fn restore(&mut self, _: &NetworkConfiguration) -> Result<(), NetworkError> {
            Err(NetworkError::Unavailable)
        }
    }
    let mut store = claimed();
    let mut network = LostNetwork { applied: false };
    let result = execute(
        &mut store,
        &mut network,
        &key(1),
        OperationId(1),
        NetworkIntent::EnableAccessPoint(profile("ap")),
    )
    .unwrap();
    assert!(matches!(
        result.status,
        OperationStatus::NeedsReconciliation { .. }
    ));
    network.applied = false;
    assert_eq!(
        execute(
            &mut store,
            &mut network,
            &key(1),
            OperationId(2),
            NetworkIntent::DisableAccessPoint
        ),
        Err(ApplicationError::Busy)
    );
    reconcile(&mut store, &mut network).unwrap();
    execute(
        &mut store,
        &mut network,
        &key(1),
        OperationId(2),
        NetworkIntent::DisableAccessPoint,
    )
    .unwrap();
}
