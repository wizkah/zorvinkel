use super::*;

/// In-memory NetworkManager: activation succeeds unless `reject_activation` is set, which
/// leaves the link as it was (like a wrong password).
#[derive(Default)]
struct Fake {
    connections: Vec<ConnectionInfo>,
    active: Option<String>,
    autoconnect: bool,
    access_point: bool,
    reject_activation: bool,
    calls: Vec<String>,
    next: u32,
}
impl Fake {
    fn with_access_point() -> Self {
        Self {
            access_point: true,
            ..Self::default()
        }
    }
    fn add(&mut self, name: &str, ssid: &str, mode: Mode, saved: bool) -> String {
        self.next += 1;
        let uuid = format!("uuid-{}", self.next);
        self.connections.push(ConnectionInfo {
            uuid: uuid.clone(),
            name: name.into(),
            ssid: ssid.into(),
            mode,
            saved,
        });
        uuid
    }
    fn names(&self) -> Vec<&str> {
        self.connections
            .iter()
            .map(|info| info.name.as_str())
            .collect()
    }
}
impl NetworkManagerApi for Fake {
    fn access_point_capable(&mut self) -> Result<bool, NetworkError> {
        Ok(self.access_point)
    }
    fn link(&mut self) -> Result<Link, NetworkError> {
        Ok(self
            .active
            .as_ref()
            .and_then(|uuid| self.connections.iter().find(|info| &info.uuid == uuid))
            .map_or(Link::Down, |info| Link::Activated {
                uuid: info.uuid.clone(),
                ssid: info.ssid.clone(),
                mode: info.mode,
            }))
    }
    fn connections(&mut self) -> Result<Vec<ConnectionInfo>, NetworkError> {
        Ok(self.connections.clone())
    }
    fn add_connection(&mut self, spec: &ConnectionSpec) -> Result<String, NetworkError> {
        // The password must reach NetworkManager, but never the profile's visible data.
        assert!(!spec.password.is_empty());
        self.calls.push(format!("add {}", spec.name));
        Ok(self.add(spec.name, spec.ssid, spec.mode, false))
    }
    fn save_connection(&mut self, uuid: &str) -> Result<(), NetworkError> {
        self.calls.push(format!("save {uuid}"));
        let info = self
            .connections
            .iter_mut()
            .find(|info| info.uuid == uuid)
            .ok_or(NetworkError::Rejected)?;
        info.saved = true;
        Ok(())
    }
    fn delete_connection(&mut self, uuid: &str) -> Result<(), NetworkError> {
        self.calls.push(format!("delete {uuid}"));
        self.connections.retain(|info| info.uuid != uuid);
        if self.active.as_deref() == Some(uuid) {
            self.active = None;
        }
        Ok(())
    }
    fn activate(&mut self, uuid: &str) -> Result<(), NetworkError> {
        self.calls.push(format!("activate {uuid}"));
        if !self.reject_activation {
            self.active = Some(uuid.into());
        }
        Ok(())
    }
    fn disconnect(&mut self) -> Result<(), NetworkError> {
        self.calls.push("disconnect".into());
        self.active = None;
        self.autoconnect = false;
        Ok(())
    }
    fn set_autoconnect(&mut self, enabled: bool) -> Result<(), NetworkError> {
        self.autoconnect = enabled;
        Ok(())
    }
    fn scan(&mut self) -> Result<Vec<String>, NetworkError> {
        Ok(vec!["Home".into()])
    }
}
fn adapter(fake: Fake) -> NmNetworkControl<Fake> {
    let mut adapter = NmNetworkControl::new(fake);
    adapter.connection_timeout = Duration::ZERO;
    adapter
}
fn wifi(adapter: &mut NmNetworkControl<Fake>, uuid: &str) -> NetworkConfiguration {
    let info = adapter
        .api
        .connections
        .iter()
        .find(|info| info.uuid == uuid)
        .unwrap();
    NetworkConfiguration::Wifi(profile_id(uuid, &info.ssid).unwrap())
}

#[test]
fn prepared_profiles_are_named_in_memory_and_keep_secrets_out_of_the_id() {
    let mut adapter = adapter(Fake::default());
    let profile = adapter.prepare_wifi("Event", "test-passphrase").unwrap();
    assert!(!profile.as_str().contains("test-passphrase"));
    let [info] = &adapter.api.connections[..] else {
        panic!("one profile expected");
    };
    assert!(info.name.starts_with(NAME_PREFIX) && !info.saved);
    assert_eq!(info.mode, Mode::Station);
    assert_eq!(
        adapter.prepare_wifi("Event", "short"),
        Err(NetworkError::InvalidInput)
    );
}

#[test]
fn connecting_activates_the_new_profile_and_observes_it() {
    let mut fake = Fake::default();
    let home = fake.add("Home", "Home", Mode::Station, true);
    fake.active = Some(home);
    let mut adapter = adapter(fake);
    let profile = adapter.prepare_wifi("Event", "test-passphrase").unwrap();
    let intent = NetworkIntent::ConnectWifi(profile.clone());
    adapter.apply(&intent).unwrap();
    assert_eq!(adapter.observe(), Ok(NetworkConfiguration::Wifi(profile)));
}

#[test]
fn rejected_connection_times_out_and_restore_returns_to_the_previous_network() {
    let mut fake = Fake::default();
    let home = fake.add("Home", "Home", Mode::Station, true);
    fake.active = Some(home.clone());
    let mut adapter = adapter(fake);
    let previous = adapter.observe().unwrap();
    let profile = adapter.prepare_wifi("Event", "wrong-passphrase").unwrap();
    adapter.api.reject_activation = true;
    assert_eq!(
        adapter.apply(&NetworkIntent::ConnectWifi(profile)),
        Err(NetworkError::TimedOut)
    );
    adapter.api.active = None;
    adapter.api.reject_activation = false;
    adapter.restore(&previous).unwrap();
    assert_eq!(adapter.observe(), Ok(previous));
    assert!(adapter.api.autoconnect);
}

#[test]
fn unknown_changed_and_mismatched_profiles_are_rejected() {
    let mut fake = Fake::default();
    let foreign = fake.add("Cafe", "Cafe", Mode::Station, true);
    let mut adapter = adapter(fake);
    let stale = profile_id("uuid-99", "Cafe").unwrap();
    assert_eq!(
        adapter.check_support(&NetworkIntent::ConnectWifi(stale)),
        Err(NetworkError::Rejected)
    );
    // The SSID in the id must still match, since UUIDs can be reused.
    let other_ssid = profile_id(&foreign, "Other").unwrap();
    assert_eq!(
        adapter.check_support(&NetworkIntent::ConnectWifi(other_ssid)),
        Err(NetworkError::Rejected)
    );
    let ap = profile_id(&foreign, "Cafe").unwrap();
    assert_eq!(
        adapter.check_support(&NetworkIntent::EnableAccessPoint(ap)),
        Err(NetworkError::Unsupported)
    );
    assert_eq!(
        adapter.check_support(&NetworkIntent::ConnectWifi(
            ProfileId::new("wpa:1:ab".into()).unwrap()
        )),
        Err(NetworkError::InvalidInput)
    );
}

#[test]
fn discarding_removes_only_inactive_owned_profiles() {
    let mut fake = Fake::default();
    fake.add("Cafe", "Cafe", Mode::Station, true);
    let mut adapter = adapter(fake);
    let first = adapter.prepare_wifi("Event", "test-passphrase").unwrap();
    adapter.prepare_wifi("Other", "test-passphrase").unwrap();
    adapter.apply(&NetworkIntent::ConnectWifi(first)).unwrap();
    adapter.discard_inactive_profiles().unwrap();
    let names = adapter.api.names();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"Cafe"));
    assert!(adapter.owned_connection_active().unwrap());
}

#[test]
fn persisting_saves_the_active_profile_releases_it_and_replaces_older_copies() {
    let mut fake = Fake::default();
    let foreign = fake.add("Event", "Event", Mode::Station, true);
    let older = fake.add("zorvinkel-1-1", "Event", Mode::Station, true);
    let mut adapter = adapter(fake);
    let profile = adapter.prepare_wifi("Event", "test-passphrase").unwrap();
    adapter.apply(&NetworkIntent::ConnectWifi(profile)).unwrap();
    adapter.persist_active().unwrap();
    assert!(adapter.owned.is_empty());
    let connections = &adapter.api.connections;
    assert!(connections.iter().any(|info| info.uuid == foreign));
    assert!(connections.iter().all(|info| info.uuid != older));
    assert!(connections.iter().all(|info| info.saved));
    // A saved profile is no longer owned, so cleanup never deletes it from disk.
    adapter.discard_inactive_profiles().unwrap();
    assert_eq!(adapter.api.connections.len(), 2);
}

#[test]
fn persisting_requires_an_active_owned_station_profile() {
    let mut fake = Fake::default();
    let home = fake.add("Home", "Home", Mode::Station, true);
    fake.active = Some(home);
    let mut adapter = adapter(fake);
    assert_eq!(adapter.persist_active(), Err(NetworkError::Rejected));
    adapter.api.active = None;
    assert_eq!(adapter.persist_active(), Err(NetworkError::Rejected));
    assert!(
        !adapter
            .api
            .calls
            .iter()
            .any(|call| call.starts_with("save"))
    );
}

#[test]
fn access_point_needs_the_capability_and_uses_its_own_profile_kind() {
    let mut adapter = adapter(Fake::default());
    assert!(!adapter.access_point_supported());
    assert_eq!(
        adapter.prepare_access_point("Zorvinkel-Box", "password-ok"),
        Err(NetworkError::Unsupported)
    );
    assert_eq!(
        adapter.check_support(&NetworkIntent::DisableAccessPoint),
        Err(NetworkError::Unsupported)
    );

    let mut adapter = self::adapter(Fake::with_access_point());
    let profile = adapter
        .prepare_access_point("Zorvinkel-Box", "password-ok")
        .unwrap();
    assert_eq!(
        adapter.check_support(&NetworkIntent::ConnectWifi(profile.clone())),
        Err(NetworkError::Unsupported)
    );
    adapter
        .apply(&NetworkIntent::EnableAccessPoint(profile.clone()))
        .unwrap();
    assert_eq!(
        adapter.observe(),
        Ok(NetworkConfiguration::AccessPoint(profile))
    );
}

#[test]
fn switching_off_an_own_access_point_disconnects_before_deleting() {
    let mut adapter = adapter(Fake::with_access_point());
    let profile = adapter
        .prepare_access_point("Zorvinkel-Box", "password-ok")
        .unwrap();
    adapter
        .apply(&NetworkIntent::EnableAccessPoint(profile))
        .unwrap();
    adapter.api.calls.clear();
    adapter.apply(&NetworkIntent::DisableAccessPoint).unwrap();
    assert_eq!(adapter.observe(), Ok(NetworkConfiguration::Disconnected));
    assert!(adapter.api.connections.is_empty());
    assert_eq!(adapter.api.calls[0], "disconnect");
    assert!(!adapter.api.autoconnect);
    adapter.reconnect().unwrap();
    assert!(adapter.api.autoconnect);
}

#[test]
fn an_access_point_from_before_a_restart_is_adopted_but_a_foreign_one_is_not() {
    let mut fake = Fake::with_access_point();
    let ours = fake.add("zorvinkel-7-7", "Zorvinkel-Box", Mode::AccessPoint, false);
    fake.active = Some(ours);
    let mut adapter = adapter(fake);
    adapter.apply(&NetworkIntent::DisableAccessPoint).unwrap();
    assert!(adapter.api.connections.is_empty());

    let mut fake = Fake::with_access_point();
    let foreign = fake.add("Hotspot", "Hotspot", Mode::AccessPoint, true);
    fake.active = Some(foreign);
    let mut adapter = self::adapter(fake);
    assert_eq!(
        adapter.apply(&NetworkIntent::DisableAccessPoint),
        Err(NetworkError::Unsupported)
    );
    assert_eq!(adapter.api.connections.len(), 1);
    assert!(adapter.api.active.is_some());
}

#[test]
fn preparing_removes_stale_in_memory_profiles_but_not_saved_or_active_ones() {
    let mut fake = Fake::default();
    fake.add("zorvinkel-1-1", "Old", Mode::Station, false);
    fake.add("zorvinkel-2-2", "Kept", Mode::Station, true);
    let active = fake.add("zorvinkel-3-3", "Live", Mode::Station, false);
    fake.active = Some(active);
    fake.add("Cafe", "Cafe", Mode::Station, false);
    let mut adapter = adapter(fake);
    adapter.prepare_wifi("Event", "test-passphrase").unwrap();
    let names = adapter.api.names();
    assert!(!names.contains(&"zorvinkel-1-1"));
    assert!(names.contains(&"zorvinkel-2-2"));
    assert!(names.contains(&"zorvinkel-3-3"));
    assert!(names.contains(&"Cafe"));
}

#[test]
fn reverting_drops_every_volatile_profile_of_the_service_and_frees_autoconnect() {
    let mut fake = Fake::default();
    fake.add("Cafe", "Cafe", Mode::Station, false);
    fake.add("zorvinkel-2-2", "Kept", Mode::Station, true);
    let mut adapter = adapter(fake);
    let profile = adapter.prepare_wifi("Event", "test-passphrase").unwrap();
    adapter.apply(&NetworkIntent::ConnectWifi(profile)).unwrap();
    adapter.revert_volatile().unwrap();
    assert_eq!(adapter.api.names(), ["Cafe", "zorvinkel-2-2"]);
    assert_eq!(adapter.observe(), Ok(NetworkConfiguration::Disconnected));
    assert!(adapter.api.autoconnect && adapter.owned.is_empty());
}

#[test]
fn owned_connection_is_only_reported_for_the_services_own_profiles() {
    let mut fake = Fake::default();
    let home = fake.add("Home", "Home", Mode::Station, true);
    fake.active = Some(home.clone());
    let mut adapter = adapter(fake);
    assert!(!adapter.owned_connection_active().unwrap());
    let own = adapter.prepare_wifi("Event", "test-passphrase").unwrap();
    adapter.apply(&NetworkIntent::ConnectWifi(own)).unwrap();
    assert!(adapter.owned_connection_active().unwrap());
    assert_ne!(wifi(&mut adapter, &home), adapter.observe().unwrap());
}

#[test]
fn status_reports_station_access_point_and_disconnected() {
    let mut fake = Fake::default();
    assert_eq!(network_status(&mut fake), Ok(NetworkStatus::Disconnected));
    let station = fake.add("Home", "Home", Mode::Station, true);
    fake.active = Some(station);
    assert_eq!(
        network_status(&mut fake),
        Ok(NetworkStatus::WifiConnected {
            ssid: "Home".into()
        })
    );
    let ap = fake.add("Box", "Box", Mode::AccessPoint, true);
    fake.active = Some(ap);
    assert_eq!(
        network_status(&mut fake),
        Ok(NetworkStatus::AccessPointActive { ssid: "Box".into() })
    );
    let bad = fake.add("Bad", "Bad\nSsid", Mode::Station, true);
    fake.active = Some(bad);
    assert_eq!(network_status(&mut fake), Err(PortError::Unavailable));
}

#[test]
fn a_saved_station_profile_is_found_by_its_ssid() {
    let mut fake = Fake::default();
    fake.add("Event", "Event", Mode::Station, false);
    fake.add("Hotspot", "Home", Mode::AccessPoint, true);
    let home = fake.add("Home", "Home", Mode::Station, true);
    let mut adapter = adapter(fake);
    assert_eq!(
        adapter.saved_wifi_profile("Home").unwrap(),
        Some(profile_id(&home, "Home").unwrap())
    );
    // A profile that exists only in memory is not saved.
    assert_eq!(adapter.saved_wifi_profile("Event").unwrap(), None);
    assert_eq!(adapter.saved_wifi_profile("Unbekannt").unwrap(), None);
}

#[test]
fn keeping_the_active_connection_saves_only_profiles_this_service_created() {
    let mut fake = Fake::default();
    let home = fake.add("Home", "Home", Mode::Station, true);
    fake.active = Some(home);
    let mut adapter = adapter(fake);
    adapter.keep_active().unwrap();
    assert!(
        !adapter
            .api
            .calls
            .iter()
            .any(|call| call.starts_with("save"))
    );
    let profile = adapter.prepare_wifi("Event", "test-passphrase").unwrap();
    adapter.apply(&NetworkIntent::ConnectWifi(profile)).unwrap();
    adapter.keep_active().unwrap();
    assert!(
        adapter
            .api
            .calls
            .iter()
            .any(|call| call.starts_with("save"))
    );
    adapter.api.active = None;
    assert_eq!(adapter.keep_active(), Err(NetworkError::Rejected));
}
