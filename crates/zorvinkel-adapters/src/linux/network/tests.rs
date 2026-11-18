use super::*;
use std::collections::VecDeque;

struct Script(
    VecDeque<(String, Result<String, NetworkError>)>,
    &'static str,
);
impl Script {
    fn new(steps: &[(&str, &str)]) -> Self {
        Self(
            steps
                .iter()
                .map(|(request, reply)| ((*request).into(), Ok((*reply).into())))
                .collect(),
            "0\n",
        )
    }
}
impl WpaControl for Script {
    fn request(&mut self, command: &str) -> Result<String, NetworkError> {
        if command.starts_with("GET_NETWORK ") && command.ends_with(" mode") {
            return Ok(self.1.into());
        }
        let (expected, response) = self.0.pop_front().expect("unexpected request");
        // Avoid displaying a command that might contain credentials on failure.
        if expected == "<set marker>" {
            assert!(
                command.starts_with("SET_NETWORK ") && command.contains(" id_str \"zorvinkel-")
            );
        } else {
            assert!(expected == command, "unexpected control command");
        }
        response
    }
}
fn adapter(steps: &[(&str, &str)]) -> WpaNetworkControl<Script> {
    let mut adapter = WpaNetworkControl::new(Script::new(steps));
    adapter.connection_timeout = Duration::ZERO;
    adapter
}
const HOME: &str = "wpa_state=COMPLETED\nid=0\nssid=Home\nmode=station\n";
const NEW: &str = "wpa_state=COMPLETED\nid=1\nssid=Event\nmode=station\n";

#[test]
fn temporary_profile_uses_no_save_and_keeps_secrets_out_of_process_arguments() {
    let mut adapter = adapter(&[
        ("ADD_NETWORK", "1\n"),
        ("<set marker>", "OK\n"),
        ("SET_NETWORK 1 ssid 4576656e74", "OK\n"),
        ("SET_NETWORK 1 psk \"test-passphrase\"", "OK\n"),
        ("SET_NETWORK 1 key_mgmt WPA-PSK", "OK\n"),
        ("SET_NETWORK 1 proto RSN", "OK\n"),
        ("SET_NETWORK 1 scan_ssid 1", "OK\n"),
    ]);
    assert_eq!(
        adapter.prepare_wifi("Event", "test-passphrase"),
        profile_id(1, "Event")
    );
    assert!(adapter.control.0.is_empty());
}
#[test]
fn malformed_credentials_never_reach_the_service() {
    let mut adapter = adapter(&[]);
    assert_eq!(
        adapter.prepare_wifi("Event\nSAVE_CONFIG", "test-passphrase"),
        Err(NetworkError::InvalidInput)
    );
    assert_eq!(
        adapter.prepare_wifi("Event", "short"),
        Err(NetworkError::InvalidInput)
    );
}
#[test]
fn configuration_failure_removes_only_the_new_profile() {
    let mut adapter = adapter(&[
        ("ADD_NETWORK", "7\n"),
        ("<set marker>", "OK\n"),
        ("SET_NETWORK 7 ssid 4576656e74", "FAIL\n"),
        ("REMOVE_NETWORK 7", "OK\n"),
    ]);
    assert_eq!(
        adapter.prepare_wifi("Event", "test-passphrase"),
        Err(NetworkError::Rejected)
    );
    assert!(adapter.owned.is_empty());
    assert!(adapter.control.0.is_empty());
}
#[test]
fn failed_association_restores_connection_and_enabled_flags() {
    let mut adapter = adapter(&[
        ("GET_NETWORK 1 ssid", "\"Event\"\n"),
        ("GET_NETWORK 1 ssid", "\"Event\"\n"),
        (
            "LIST_NETWORKS",
            "network id / ssid / bssid / flags\n0\tHome\tany\t[CURRENT]\n2\tOther\tany\t[DISABLED]\n1\tEvent\tany\t[DISABLED]\n",
        ),
        ("SELECT_NETWORK 1", "OK\n"),
        ("STATUS", "wpa_state=4WAY_HANDSHAKE\n"),
        ("GET_NETWORK 0 ssid", "\"Home\"\n"),
        ("SELECT_NETWORK 0", "OK\n"),
        ("GET_NETWORK 0 ssid", "\"Home\"\n"),
        ("ENABLE_NETWORK 0", "OK\n"),
        ("GET_NETWORK 2 ssid", "\"Other\"\n"),
        ("DISABLE_NETWORK 2", "OK\n"),
        ("GET_NETWORK 1 ssid", "\"Event\"\n"),
        ("DISABLE_NETWORK 1", "OK\n"),
        ("STATUS", HOME),
    ]);
    assert_eq!(
        adapter.apply(&NetworkIntent::ConnectWifi(profile_id(1, "Event").unwrap())),
        Err(NetworkError::TimedOut)
    );
    adapter
        .restore(&NetworkConfiguration::Wifi(profile_id(0, "Home").unwrap()))
        .unwrap();
    assert!(adapter.enabled_before.is_none());
    assert!(adapter.control.0.is_empty());
}
#[test]
fn completed_association_does_not_claim_ip_reachability() {
    let mut adapter = adapter(&[
        ("GET_NETWORK 1 ssid", "4576656e74\n"),
        ("GET_NETWORK 1 ssid", "4576656e74\n"),
        (
            "LIST_NETWORKS",
            "network id / ssid / bssid / flags\n1\tEvent\tany\t[DISABLED]\n",
        ),
        ("SELECT_NETWORK 1", "OK\n"),
        ("STATUS", NEW),
    ]);
    adapter
        .apply(&NetworkIntent::ConnectWifi(profile_id(1, "Event").unwrap()))
        .unwrap();
    assert!(adapter.control.0.is_empty());
}
#[test]
fn reused_id_with_different_ssid_is_not_selected() {
    let mut adapter = adapter(&[("GET_NETWORK 0 ssid", "\"Other\"\n")]);
    assert_eq!(
        adapter.apply(&NetworkIntent::ConnectWifi(profile_id(0, "Home").unwrap())),
        Err(NetworkError::Rejected)
    );
    assert!(adapter.control.0.is_empty());
}
#[test]
fn ap_and_malformed_status_are_explicit_errors() {
    let mut adapter = adapter(&[("STATUS", "FAIL\n")]);
    assert_eq!(
        adapter.check_support(&NetworkIntent::EnableAccessPoint(
            profile_id(0, "AP").unwrap()
        )),
        Err(NetworkError::Unsupported)
    );
    assert_eq!(adapter.observe(), Err(NetworkError::Unavailable));
}
#[test]
fn scans_decode_utf8_deduplicate_and_reject_bad_replies() {
    let mut adapter = adapter(&[
        ("SCAN", "OK\n"),
        (
            "SCAN_RESULTS",
            "bssid / frequency / signal level / flags / ssid\na\t2412\t-42\t[WPA2]\tCaf\\xc3\\xa9\nb\t2412\t-50\t[WPA2]\tCaf\\xc3\\xa9\n",
        ),
        ("SCAN_RESULTS", "FAIL\n"),
    ]);
    adapter.request_scan().unwrap();
    assert_eq!(adapter.cached_scan_results().unwrap(), vec!["Café"]);
    assert_eq!(
        adapter.cached_scan_results(),
        Err(NetworkError::Unavailable)
    );
}
#[test]
fn active_temporary_profile_cannot_be_discarded() {
    let mut adapter = adapter(&[("STATUS", NEW)]);
    adapter.owned.insert(1, profile_id(1, "Event").unwrap());
    assert_eq!(adapter.discard_profiles(), Err(NetworkError::Rejected));
}
#[test]
fn socket_transport_has_private_paths_and_cleans_up() {
    use std::{
        fs,
        os::unix::{fs::PermissionsExt, net::UnixDatagram},
    };
    let root = std::env::temp_dir().join(format!("zv-wpa-test-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let server_path = root.join("server");
    let server = UnixDatagram::bind(&server_path).unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let task = thread::spawn(move || {
        let mut buffer = [0; 64];
        let (length, peer) = server.recv_from(&mut buffer).unwrap();
        assert_eq!(&buffer[..length], b"STATUS");
        server
            .send_to(HOME.as_bytes(), peer.as_pathname().unwrap())
            .unwrap();
    });
    let mut socket = WpaSocket::open(&server_path, &root).unwrap();
    let local = fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.is_dir())
        .unwrap();
    assert_eq!(
        fs::metadata(&local).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(socket.request("STATUS").unwrap(), HOME);
    task.join().unwrap();
    drop(socket);
    assert!(!local.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cleanup_rejects_reused_id_even_with_identical_ssid() {
    let mut adapter = adapter(&[
        ("STATUS", HOME),
        ("GET_NETWORK 1 id_str", "\"different-owner\"\n"),
    ]);
    adapter.owned.insert(1, profile_id(1, "Event").unwrap());
    adapter.markers.insert(1, "original-owner".into());
    assert_eq!(adapter.discard_profiles(), Err(NetworkError::Rejected));
    assert!(adapter.control.0.is_empty());
}

#[test]
fn fresh_scan_waits_for_monitor_completion_before_reading_results() {
    use std::{fs, os::unix::net::UnixDatagram};
    let root = std::env::temp_dir().join(format!("zv-scan-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let server_path = root.join("server");
    let server = UnixDatagram::bind(&server_path).unwrap();
    server
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let task = thread::spawn(move || {
        let mut buffer = [0; 4096];
        let (length, monitor) = server.recv_from(&mut buffer).unwrap();
        assert_eq!(&buffer[..length], b"ATTACH");
        server
            .send_to(b"OK\n", monitor.as_pathname().unwrap())
            .unwrap();
        let (length, command) = server.recv_from(&mut buffer).unwrap();
        assert_eq!(&buffer[..length], b"SCAN");
        server
            .send_to(b"OK\n", command.as_pathname().unwrap())
            .unwrap();
        server
            .send_to(
                b"<3>CTRL-EVENT-SCAN-STARTED\n",
                monitor.as_pathname().unwrap(),
            )
            .unwrap();
        server
            .send_to(
                b"<3>CTRL-EVENT-SCAN-RESULTS\n",
                monitor.as_pathname().unwrap(),
            )
            .unwrap();
        let (length, _) = server.recv_from(&mut buffer).unwrap();
        assert_eq!(&buffer[..length], b"DETACH");
        let (length, command) = server.recv_from(&mut buffer).unwrap();
        assert_eq!(&buffer[..length], b"SCAN_RESULTS");
        server
            .send_to(
                b"bssid / frequency / signal level / flags / ssid\na\t2412\t-40\t[WPA2]\tEvent\n",
                command.as_pathname().unwrap(),
            )
            .unwrap();
    });
    let socket = WpaSocket::open(&server_path, &root).unwrap();
    let mut network = WpaNetworkControl::new(socket);
    assert_eq!(network.scan().unwrap(), vec!["Event"]);
    task.join().unwrap();
    drop(network);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn persistence_reenables_foreign_profiles_before_saving_only_the_active_profile() {
    let mut adapter = adapter(&[
        ("STATUS", NEW),
        ("GET_NETWORK 1 ssid", "\"Event\"\n"),
        ("GET_NETWORK 1 id_str", "\"zorvinkel-test\"\n"),
        ("STATUS", NEW),
        ("GET_NETWORK 0 ssid", "\"Home\"\n"),
        ("ENABLE_NETWORK 0", "OK\n"),
        ("STATUS", NEW),
        ("SAVE_CONFIG", "OK\n"),
    ]);
    adapter.owned.insert(1, profile_id(1, "Event").unwrap());
    adapter.markers.insert(1, "zorvinkel-test".into());
    adapter.enabled_before = Some(vec![
        (profile_id(0, "Home").unwrap(), true),
        (profile_id(1, "Event").unwrap(), true),
    ]);
    assert_eq!(adapter.persist_active(), Ok(()));
    assert!(adapter.control.0.is_empty());
}
#[test]
fn persistence_refuses_foreign_or_changed_connections() {
    let mut foreign = adapter(&[("STATUS", HOME), ("GET_NETWORK 0 ssid", "\"Home\"\n")]);
    assert_eq!(foreign.persist_active(), Err(NetworkError::Rejected));
    let mut roamed = adapter(&[
        ("STATUS", NEW),
        ("GET_NETWORK 1 ssid", "\"Event\"\n"),
        ("GET_NETWORK 1 id_str", "\"zorvinkel-test\"\n"),
        ("STATUS", NEW),
        ("GET_NETWORK 0 ssid", "\"Home\"\n"),
        ("ENABLE_NETWORK 0", "OK\n"),
        ("STATUS", HOME),
    ]);
    roamed.owned.insert(1, profile_id(1, "Event").unwrap());
    roamed.markers.insert(1, "zorvinkel-test".into());
    roamed.enabled_before = Some(vec![(profile_id(0, "Home").unwrap(), true)]);
    assert_eq!(roamed.persist_active(), Err(NetworkError::Rejected));
}

#[test]
fn revert_reloads_the_saved_configuration_and_forgets_volatile_profiles() {
    let mut saved = adapter(&[("RECONFIGURE", "OK\n")]);
    saved.owned.insert(1, profile_id(1, "Event").unwrap());
    saved.markers.insert(1, "zorvinkel-test".into());
    saved.enabled_before = Some(vec![(profile_id(0, "Home").unwrap(), true)]);
    assert_eq!(saved.revert_volatile(), Ok(()));
    assert!(saved.control.0.is_empty());
    assert!(saved.owned.is_empty() && saved.enabled_before.is_none());
    let mut rejected = adapter(&[("RECONFIGURE", "FAIL\n")]);
    assert_eq!(rejected.revert_volatile(), Err(NetworkError::Rejected));
}

mod access_point {
    use super::*;
    use std::sync::{Arc, Mutex};

    type Calls = Arc<Mutex<Vec<&'static str>>>;
    struct Link(Calls, bool);
    impl AccessPointLink for Link {
        fn up(&mut self) -> Result<(), NetworkError> {
            self.0.lock().unwrap().push("up");
            if self.1 {
                Ok(())
            } else {
                Err(NetworkError::Rejected)
            }
        }
        fn down(&mut self) -> Result<(), NetworkError> {
            self.0.lock().unwrap().push("down");
            Ok(())
        }
    }
    fn with_link(steps: &[(&str, &str)], link_works: bool) -> (WpaNetworkControl<Script>, Calls) {
        let calls = Calls::default();
        let mut adapter =
            adapter(steps).with_access_point(Box::new(Link(calls.clone(), link_works)));
        adapter.control.1 = "2\n";
        (adapter, calls)
    }
    const AP: &str = "wpa_state=COMPLETED\nid=1\nssid=Box\nmode=AP\n";

    #[test]
    fn profile_is_a_volatile_wpa2_ccmp_access_point_and_needs_the_helper() {
        let (mut adapter, _) = with_link(
            &[
                ("ADD_NETWORK", "1\n"),
                ("<set marker>", "OK\n"),
                ("SET_NETWORK 1 ssid 426f78", "OK\n"),
                ("SET_NETWORK 1 psk \"test-passphrase\"", "OK\n"),
                ("SET_NETWORK 1 key_mgmt WPA-PSK", "OK\n"),
                ("SET_NETWORK 1 proto RSN", "OK\n"),
                ("SET_NETWORK 1 mode 2", "OK\n"),
                ("SET_NETWORK 1 pairwise CCMP", "OK\n"),
                ("SET_NETWORK 1 group CCMP", "OK\n"),
                ("SET_NETWORK 1 frequency 2437", "OK\n"),
            ],
            true,
        );
        assert_eq!(
            adapter.prepare_access_point("Box", "test-passphrase"),
            profile_id(1, "Box")
        );
        assert!(adapter.control.0.is_empty());
        let mut without_helper = super::adapter(&[]);
        assert_eq!(
            without_helper.prepare_access_point("Box", "test-passphrase"),
            Err(NetworkError::Unsupported)
        );
        assert_eq!(
            adapter.prepare_access_point("Box", "ünicode-passphrase"),
            Err(NetworkError::InvalidInput)
        );
    }
    #[test]
    fn enabling_selects_the_profile_waits_for_ap_mode_and_brings_the_link_up() {
        let (mut adapter, calls) = with_link(
            &[
                ("GET_NETWORK 1 ssid", "\"Box\"\n"),
                ("GET_NETWORK 1 ssid", "\"Box\"\n"),
                (
                    "LIST_NETWORKS",
                    "network id / ssid / bssid / flags\n0\tHome\tany\t[CURRENT]\n1\tBox\tany\t[DISABLED]\n",
                ),
                ("SELECT_NETWORK 1", "OK\n"),
                ("STATUS", AP),
            ],
            true,
        );
        adapter
            .apply(&NetworkIntent::EnableAccessPoint(
                profile_id(1, "Box").unwrap(),
            ))
            .unwrap();
        assert_eq!(*calls.lock().unwrap(), ["up"]);
        assert!(adapter.control.0.is_empty());
    }
    #[test]
    fn failing_link_tears_the_half_started_access_point_down() {
        let (mut adapter, calls) = with_link(
            &[
                ("GET_NETWORK 1 ssid", "\"Box\"\n"),
                ("GET_NETWORK 1 ssid", "\"Box\"\n"),
                (
                    "LIST_NETWORKS",
                    "network id / ssid / bssid / flags\n1\tBox\tany\t[DISABLED]\n",
                ),
                ("SELECT_NETWORK 1", "OK\n"),
                ("STATUS", AP),
            ],
            false,
        );
        assert_eq!(
            adapter.apply(&NetworkIntent::EnableAccessPoint(
                profile_id(1, "Box").unwrap()
            )),
            Err(NetworkError::Rejected)
        );
        assert_eq!(*calls.lock().unwrap(), ["up", "down"]);
    }
    #[test]
    fn status_in_ap_mode_is_an_access_point_configuration() {
        let (mut adapter, _) = with_link(&[("STATUS", AP)], true);
        assert_eq!(
            adapter.observe(),
            Ok(NetworkConfiguration::AccessPoint(
                profile_id(1, "Box").unwrap()
            ))
        );
        let mut other = super::adapter(&[(
            "STATUS",
            "wpa_state=COMPLETED\nid=1\nssid=Box\nmode=P2P GO\n",
        )]);
        assert_eq!(other.observe(), Err(NetworkError::Unsupported));
    }
    #[test]
    fn disabling_removes_only_an_owned_profile_and_leaves_the_radio_disconnected() {
        let (mut adapter, calls) = with_link(
            &[
                ("STATUS", AP),
                ("GET_NETWORK 1 ssid", "\"Box\"\n"),
                ("GET_NETWORK 1 id_str", "\"zorvinkel-old-process\"\n"),
                ("GET_NETWORK 1 id_str", "\"zorvinkel-old-process\"\n"),
                ("GET_NETWORK 1 ssid", "\"Box\"\n"),
                ("GET_NETWORK 1 id_str", "\"zorvinkel-old-process\"\n"),
                ("REMOVE_NETWORK 1", "OK\n"),
                ("DISCONNECT", "OK\n"),
                ("STATUS", "wpa_state=DISCONNECTED\n"),
            ],
            true,
        );
        adapter.apply(&NetworkIntent::DisableAccessPoint).unwrap();
        assert_eq!(*calls.lock().unwrap(), ["down"]);
        assert!(adapter.owned.is_empty());
        assert!(adapter.control.0.is_empty());

        let (mut foreign, calls) = with_link(
            &[
                ("STATUS", AP),
                ("GET_NETWORK 1 ssid", "\"Box\"\n"),
                ("GET_NETWORK 1 id_str", "\"someone-else\"\n"),
            ],
            true,
        );
        assert_eq!(
            foreign.apply(&NetworkIntent::DisableAccessPoint),
            Err(NetworkError::Unsupported)
        );
        assert_eq!(*calls.lock().unwrap(), ["down"]);
    }
    #[test]
    fn revert_also_stops_addressing_and_dhcp() {
        let (mut adapter, calls) = with_link(&[("RECONFIGURE", "OK\n")], true);
        adapter.revert_volatile().unwrap();
        assert_eq!(*calls.lock().unwrap(), ["down"]);
    }
}

#[test]
fn a_saved_station_profile_is_found_by_its_ssid() {
    let mut adapter = adapter(&[
        (
            "LIST_NETWORKS",
            "network id / ssid / bssid / flags\n0\tHome\tany\t[CURRENT]\n3\tAP\tany\t\n",
        ),
        ("GET_NETWORK 0 ssid", "\"Home\"\n"),
    ]);
    assert_eq!(
        adapter.saved_wifi_profile("Home").unwrap(),
        Some(profile_id(0, "Home").unwrap())
    );
    assert!(adapter.control.0.is_empty());
    // An access point profile of the same name is no station profile.
    let mut other = adapter_with_mode(
        &[
            (
                "LIST_NETWORKS",
                "network id / ssid / bssid / flags\n3\tAP\tany\t\n",
            ),
            ("GET_NETWORK 3 ssid", "\"AP\"\n"),
        ],
        "2\n",
    );
    assert_eq!(other.saved_wifi_profile("AP").unwrap(), None);
    let mut other = adapter_with_mode(
        &[(
            "LIST_NETWORKS",
            "network id / ssid / bssid / flags\n0\tHome\tany\t\n",
        )],
        "0\n",
    );
    assert_eq!(other.saved_wifi_profile("Unbekannt").unwrap(), None);
}
fn adapter_with_mode(steps: &[(&str, &str)], mode: &'static str) -> WpaNetworkControl<Script> {
    let mut stack = adapter(steps);
    stack.control.1 = mode;
    stack
}
#[test]
fn keeping_the_active_connection_saves_only_profiles_this_service_created() {
    // A profile of the saved configuration is kept as it is: nothing is written.
    let mut saved = adapter(&[("STATUS", HOME), ("GET_NETWORK 0 ssid", "\"Home\"\n")]);
    saved.keep_active().unwrap();
    assert!(saved.control.0.is_empty());
    // Without a connection there is nothing to keep.
    let mut idle = adapter(&[("STATUS", "wpa_state=DISCONNECTED\n")]);
    assert_eq!(idle.keep_active(), Err(NetworkError::Rejected));
}
