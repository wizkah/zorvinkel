use zorvinkel_adapters::simulation::setup::{DEMO_WIFI_PASSWORD, SimulatedSetup};
use zorvinkel_application::setup::*;
use zorvinkel_core::{IpReachability, NetworkStatus, network::RestartPolicy};
fn connected() -> SimulatedSetup {
    let mut backend = SimulatedSetup::default();
    let devices = backend
        .handle(SetupRequest::Discover)
        .unwrap()
        .snapshot
        .devices;
    let update = backend
        .handle(SetupRequest::Connect(devices[0].handle.clone()))
        .unwrap();
    assert!(update.error.is_none());
    backend
}
fn wifi(backend: &mut SimulatedSetup, password: &str) -> SetupUpdate {
    backend
        .handle(SetupRequest::ConnectWifi {
            ssid: "Event-WiFi".into(),
            password: password.into(),
        })
        .unwrap()
}

#[test]
fn device_commands_require_a_selected_connected_device() {
    let mut backend = SimulatedSetup::default();
    for request in [
        SetupRequest::Scan,
        SetupRequest::EnableAccessPoint(AccessPointSettings {
            ssid: "Box".into(),
            password: "password-ok".into(),
        }),
        SetupRequest::ReadSavedCredentials {
            ssid: "Demo".into(),
        },
        SetupRequest::Connect("unknown".into()),
    ] {
        let update = backend.handle(request).unwrap();
        assert_eq!(update.error, Some(SetupError::Disconnected));
        assert!(update.snapshot.access_point.is_none());
        assert!(update.snapshot.status.is_none());
    }
}

#[test]
fn failed_password_is_correctable_without_losing_local_access() {
    let mut backend = connected();
    let failed = wifi(&mut backend, "wrong-password");
    assert_eq!(failed.error, Some(SetupError::ConnectionRejected));
    assert_eq!(
        failed.snapshot.status.unwrap().network,
        NetworkStatus::Disconnected
    );
    let corrected = wifi(&mut backend, DEMO_WIFI_PASSWORD);
    assert!(corrected.error.is_none());
    let status = corrected.snapshot.status.unwrap();
    assert_eq!(
        status.network,
        NetworkStatus::WifiConnected {
            ssid: "Event-WiFi".into()
        }
    );
    assert_eq!(status.ip_reachability, IpReachability::Unverified);
}

#[test]
fn reconnect_reads_actual_network_and_restart_configuration() {
    let mut backend = connected();
    wifi(&mut backend, DEMO_WIFI_PASSWORD);
    backend
        .handle(SetupRequest::RestartCurrentAsDefault)
        .unwrap();
    let disconnected = backend.handle(SetupRequest::Disconnect).unwrap();
    assert!(disconnected.snapshot.status.is_none());
    let update = backend
        .handle(SetupRequest::Connect(
            disconnected.snapshot.devices[0].handle.clone(),
        ))
        .unwrap();
    assert!(matches!(
        update.snapshot.status.unwrap().network,
        NetworkStatus::WifiConnected { .. }
    ));
    assert!(matches!(
        update.snapshot.restart_policy,
        Some(RestartPolicy::DefaultProfile(_))
    ));
}

#[test]
fn a_change_differs_from_the_persisted_state_until_persisted_or_reverted() {
    let mut backend = connected();
    assert!(
        !backend
            .handle(SetupRequest::Refresh)
            .unwrap()
            .snapshot
            .restart_differs
    );
    let changed = wifi(&mut backend, DEMO_WIFI_PASSWORD);
    assert!(
        changed.snapshot.restart_differs,
        "volatile after the change"
    );

    let reverted = backend.handle(SetupRequest::RestartRevert).unwrap();
    assert!(reverted.error.is_none());
    assert!(!reverted.snapshot.restart_differs);
    assert_eq!(
        reverted.snapshot.status.unwrap().network,
        NetworkStatus::Disconnected,
        "back at once to the persisted configuration"
    );

    wifi(&mut backend, DEMO_WIFI_PASSWORD);
    let persisted = backend
        .handle(SetupRequest::RestartCurrentAsDefault)
        .unwrap();
    assert!(!persisted.snapshot.restart_differs);
    let again = backend.handle(SetupRequest::RestartRevert).unwrap();
    assert!(matches!(
        again.snapshot.status.unwrap().network,
        NetworkStatus::WifiConnected { .. }
    ));
}

#[test]
fn access_point_data_comes_from_the_device_and_updates_while_active() {
    let mut backend = connected();
    let proposed = backend.handle(SetupRequest::Refresh).unwrap().snapshot;
    assert!(proposed.access_point_available && proposed.access_point_proposed);
    let proposal = proposed.access_point.unwrap();
    assert_eq!(proposal.ssid, "Zorvinkel-Demo");
    assert!(
        !backend
            .handle(SetupRequest::EnableAccessPoint(proposal))
            .unwrap()
            .snapshot
            .access_point_proposed
    );
    let updated = backend
        .handle(SetupRequest::UpdateAccessPoint(AccessPointSettings {
            ssid: "New; Demo".into(),
            password: "new-demo-password".into(),
        }))
        .unwrap();
    assert!(updated.error.is_none());
    assert_eq!(updated.snapshot.access_point.unwrap().ssid, "New; Demo");
    assert_eq!(
        updated.snapshot.status.unwrap().network,
        NetworkStatus::AccessPointActive {
            ssid: "New; Demo".into()
        }
    );
    let disabled = backend.handle(SetupRequest::DisableAccessPoint).unwrap();
    assert_eq!(
        disabled.snapshot.status.unwrap().network,
        NetworkStatus::Disconnected
    );
}

#[test]
fn invalid_inputs_and_credential_failure_preserve_manual_setup() {
    let mut backend = connected();
    assert_eq!(
        wifi(&mut backend, "short").error,
        Some(SetupError::InvalidInput)
    );
    let unavailable = backend
        .handle(SetupRequest::ReadSavedCredentials {
            ssid: "Demo".into(),
        })
        .unwrap();
    assert_eq!(unavailable.error, Some(SetupError::CredentialsUnavailable));
    assert!(wifi(&mut backend, DEMO_WIFI_PASSWORD).error.is_none());
    assert!(validate_wifi(&"ü".repeat(16), DEMO_WIFI_PASSWORD).is_ok());
    assert_eq!(
        validate_wifi(&"ü".repeat(17), DEMO_WIFI_PASSWORD),
        Err(SetupError::InvalidInput)
    );
}

#[test]
fn wifi_persistence_choice_is_offered_and_changed_by_the_client() {
    let mut backend = connected();
    let update = backend.handle(SetupRequest::Refresh).unwrap();
    assert_eq!(update.snapshot.wifi_persistence, Some(false));
    let update = backend
        .handle(SetupRequest::SetWifiPersistence(true))
        .unwrap();
    assert!(update.error.is_none());
    assert_eq!(update.snapshot.wifi_persistence, Some(true));
    let update = backend
        .handle(SetupRequest::SetWifiPersistence(false))
        .unwrap();
    assert_eq!(update.snapshot.wifi_persistence, Some(false));
}

#[test]
fn power_request_ends_the_simulated_connection() {
    use zorvinkel_application::power::PowerAction;
    for action in [PowerAction::Shutdown, PowerAction::Restart] {
        let mut backend = connected();
        let update = backend.handle(SetupRequest::PowerDevice(action)).unwrap();
        assert!(update.error.is_none());
        assert!(update.snapshot.status.is_none());
        assert!(!update.snapshot.power_available);
        let update = backend.handle(SetupRequest::PowerDevice(action)).unwrap();
        assert_eq!(update.error, Some(SetupError::Disconnected));
    }
}
