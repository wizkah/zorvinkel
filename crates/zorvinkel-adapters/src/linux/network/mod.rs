//! WPA2-Personal control. Existing dhcpcd observes link changes itself.
//! Profiles are volatile unless the caller explicitly persists the active one via
//! `persist_active` (opt-in). No service restarts or automatic startup replay.
mod socket;
use super::{access_point::AccessPointLink, stack::StackControl};
pub use socket::WpaSocket;
use std::{
    collections::BTreeMap,
    thread,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;
use zorvinkel_application::{
    network::NetworkControl,
    setup::{validate_access_point, validate_wifi},
};
use zorvinkel_core::network::*;

pub trait WpaControl {
    /// Implementations must bound response size and completion time. Never log commands.
    fn scan_complete(&mut self) -> Result<(), NetworkError> {
        Err(NetworkError::Unsupported)
    }
    fn request(&mut self, command: &str) -> Result<String, NetworkError>;
}

pub struct WpaNetworkControl<C = WpaSocket> {
    control: C,
    owned: BTreeMap<u32, ProfileId>,
    markers: BTreeMap<u32, String>,
    enabled_before: Option<Vec<(ProfileId, bool)>>,
    connection_timeout: Duration,
    /// Present only where the installer set up addressing and DHCP for the access point.
    access_point: Option<Box<dyn AccessPointLink>>,
}
impl<C: WpaControl> WpaNetworkControl<C> {
    pub fn new(control: C) -> Self {
        Self {
            control,
            owned: BTreeMap::new(),
            markers: BTreeMap::new(),
            enabled_before: None,
            connection_timeout: Duration::from_secs(20),
            access_point: None,
        }
    }
    pub fn with_access_point(mut self, link: Box<dyn AccessPointLink>) -> Self {
        self.access_point = Some(link);
        self
    }
    pub fn access_point_supported(&self) -> bool {
        self.access_point.is_some()
    }

    /// Adds a disabled in-memory profile. Passwords never enter process arguments.
    pub fn prepare_wifi(&mut self, ssid: &str, password: &str) -> Result<ProfileId, NetworkError> {
        validate_wifi(ssid, password).map_err(|_| NetworkError::InvalidInput)?;
        self.prepare(ssid, password, false)
    }
    /// Adds a disabled in-memory profile that makes the interface an access point (WPA2-PSK,
    /// CCMP only, channel 6). It is never written to the saved configuration.
    pub fn prepare_access_point(
        &mut self,
        ssid: &str,
        password: &str,
    ) -> Result<ProfileId, NetworkError> {
        validate_access_point(ssid, password).map_err(|_| NetworkError::InvalidInput)?;
        if self.access_point.is_none() {
            return Err(NetworkError::Unsupported);
        }
        self.prepare(ssid, password, true)
    }
    fn prepare(
        &mut self,
        ssid: &str,
        password: &str,
        access_point: bool,
    ) -> Result<ProfileId, NetworkError> {
        if self.owned.len() >= 16 {
            return Err(NetworkError::Unavailable);
        }
        let id = self
            .control
            .request("ADD_NETWORK")?
            .trim()
            .parse::<u32>()
            .map_err(|_| NetworkError::Rejected)?;
        let profile = profile_id(id, ssid)?;
        self.owned.insert(id, profile.clone());
        let marker = format!(
            "zorvinkel-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| NetworkError::Unavailable)?
                .as_nanos()
        );
        self.markers.insert(id, marker.clone());
        let result = (|| {
            self.ok(&format!("SET_NETWORK {id} id_str \"{marker}\""))?;
            self.ok(&format!("SET_NETWORK {id} ssid {}", hex(ssid.as_bytes())))?;
            let mut escaped = Zeroizing::new(String::new());
            for character in password.chars() {
                if character == '\\' || character == '"' {
                    escaped.push('\\');
                }
                escaped.push(character);
            }
            let command = Zeroizing::new(format!("SET_NETWORK {id} psk \"{}\"", escaped.as_str()));
            self.ok(&command)?;
            self.ok(&format!("SET_NETWORK {id} key_mgmt WPA-PSK"))?;
            self.ok(&format!("SET_NETWORK {id} proto RSN"))?;
            if access_point {
                self.ok(&format!("SET_NETWORK {id} mode 2"))?;
                self.ok(&format!("SET_NETWORK {id} pairwise CCMP"))?;
                self.ok(&format!("SET_NETWORK {id} group CCMP"))?;
                self.ok(&format!("SET_NETWORK {id} frequency 2437"))
            } else {
                self.ok(&format!("SET_NETWORK {id} scan_ssid 1"))
            }
        })();
        if let Err(error) = result {
            self.ok(&format!("REMOVE_NETWORK {id}"))?;
            self.owned.remove(&id);
            self.markers.remove(&id);
            return Err(error);
        }
        Ok(profile)
    }

    pub fn scan(&mut self) -> Result<Vec<String>, NetworkError> {
        self.control.scan_complete()?;
        self.cached_scan_results()
    }
    pub fn discard_inactive_profiles(&mut self) -> Result<(), NetworkError> {
        let current = self.observe()?;
        let ids = self
            .owned
            .iter()
            .filter_map(|(id, profile)| {
                (current != NetworkConfiguration::Wifi(profile.clone())
                    && current != NetworkConfiguration::AccessPoint(profile.clone()))
                .then_some(*id)
            })
            .collect::<Vec<_>>();
        for id in ids {
            self.remove_owned(id)?;
        }
        Ok(())
    }
    /// Opt-in persistence: let wpa_supplicant write its own configuration. Only the active
    /// owned profile may remain, and previously enabled foreign profiles are re-enabled first
    /// so that SELECT_NETWORK's temporary disabling is never written to the file.
    pub fn persist_active(&mut self) -> Result<(), NetworkError> {
        let active = self.observe()?;
        let NetworkConfiguration::Wifi(active_profile) = &active else {
            return Err(NetworkError::Rejected);
        };
        let id = self.verify_profile(active_profile)?;
        if !self.owned.contains_key(&id) {
            return Err(NetworkError::Rejected);
        }
        self.discard_inactive_profiles()?;
        self.reenable_foreign_profiles(Some(active_profile))?;
        if self.observe()? != active {
            return Err(NetworkError::Rejected);
        }
        self.ok("SAVE_CONFIG")?;
        Ok(())
    }
    /// SELECT_NETWORK disables every other profile; `enabled_before` remembers which ones were
    /// enabled so they can be released again (`keep` stays untouched).
    pub fn reenable_foreign_profiles(
        &mut self,
        keep: Option<&ProfileId>,
    ) -> Result<(), NetworkError> {
        if let Some(profiles) = self.enabled_before.clone() {
            for (profile, enabled) in profiles {
                if enabled && Some(&profile) != keep {
                    let foreign = self.verify_profile(&profile)?;
                    self.ok(&format!("ENABLE_NETWORK {foreign}"))?;
                }
            }
        }
        self.enabled_before = None;
        Ok(())
    }
    /// True while the connection is one of this instance's own (volatile) profiles.
    pub fn owned_connection_active(&mut self) -> Result<bool, NetworkError> {
        let current = self.observe()?;
        Ok(self.owned.values().any(|profile| {
            current == NetworkConfiguration::Wifi(profile.clone())
                || current == NetworkConfiguration::AccessPoint(profile.clone())
        }))
    }
    /// The profile that wpa_supplicant knows for this network and that is a station profile.
    pub fn saved_wifi_profile(&mut self, ssid: &str) -> Result<Option<ProfileId>, NetworkError> {
        for (profile, _) in self.enabled_profiles()? {
            if super::stack::profile_ssid(&profile).as_deref() != Some(ssid) {
                continue;
            }
            let id = self.verify_profile(&profile)?;
            if self
                .control
                .request(&format!("GET_NETWORK {id} mode"))?
                .trim()
                == "0"
            {
                return Ok(Some(profile));
            }
        }
        Ok(None)
    }
    /// Saves the active connection when this instance created it; a profile from the saved
    /// configuration is kept as it is.
    pub fn keep_active(&mut self) -> Result<(), NetworkError> {
        let NetworkConfiguration::Wifi(profile) = self.observe()? else {
            return Err(NetworkError::Rejected);
        };
        let id = self.verify_profile(&profile)?;
        if self.owned.contains_key(&id) {
            self.persist_active()
        } else {
            Ok(())
        }
    }
    /// Lets wpa_supplicant pick a saved network again after the access point was switched off.
    pub fn reconnect(&mut self) -> Result<(), NetworkError> {
        self.ok("RECONNECT")
    }
    /// Returns to the saved configuration: wpa_supplicant re-reads its configuration file, which
    /// drops every volatile profile and enables the saved ones again. Needs no memory of the
    /// change, so it also works after the service restarted.
    pub fn revert_volatile(&mut self) -> Result<(), NetworkError> {
        let link = self.access_point.as_mut().map(|link| link.down());
        self.ok("RECONFIGURE")?;
        self.owned.clear();
        self.markers.clear();
        self.enabled_before = None;
        link.unwrap_or(Ok(()))
    }
    pub fn request_scan(&mut self) -> Result<(), NetworkError> {
        self.ok("SCAN")
    }

    /// Results are cached by wpa_supplicant; requesting a scan does not make them fresh.
    pub fn cached_scan_results(&mut self) -> Result<Vec<String>, NetworkError> {
        let response = self.control.request("SCAN_RESULTS")?;
        if !response.starts_with("bssid / frequency / signal level / flags / ssid") {
            return Err(NetworkError::Unavailable);
        }
        let mut networks = Vec::new();
        for line in response.lines().skip(1) {
            let ssid = line
                .splitn(5, '\t')
                .nth(4)
                .ok_or(NetworkError::Unavailable)?;
            let ssid = decode_ssid(ssid)?;
            if !ssid.is_empty() && !networks.contains(&ssid) {
                networks.push(ssid);
            }
        }
        Ok(networks)
    }

    /// Remove only profiles created by this instance; active profiles require restore first.
    pub fn discard_profiles(&mut self) -> Result<(), NetworkError> {
        let current = self.observe()?;
        if self
            .owned
            .values()
            .any(|p| current == NetworkConfiguration::Wifi(p.clone()))
        {
            return Err(NetworkError::Rejected);
        }
        for id in self.owned.keys().copied().collect::<Vec<_>>() {
            self.remove_owned(id)?;
        }
        Ok(())
    }

    fn remove_owned(&mut self, id: u32) -> Result<(), NetworkError> {
        let profile = self
            .owned
            .get(&id)
            .ok_or(NetworkError::InvalidInput)?
            .clone();
        // A restarted supplicant can reuse IDs, including profiles with the same SSID.
        self.verify_owned(id)?;
        self.verify_profile(&profile)?;
        self.ok(&format!("REMOVE_NETWORK {id}"))?;
        self.owned.remove(&id);
        self.markers.remove(&id);
        Ok(())
    }
    fn verify_owned(&mut self, id: u32) -> Result<(), NetworkError> {
        let marker = self.markers.get(&id).ok_or(NetworkError::Rejected)?;
        let actual = self.control.request(&format!("GET_NETWORK {id} id_str"))?;
        if actual.trim() != format!("\"{marker}\"") {
            return Err(NetworkError::Rejected);
        }
        Ok(())
    }
    fn ok(&mut self, command: &str) -> Result<(), NetworkError> {
        match self.control.request(command)?.trim() {
            "OK" => Ok(()),
            _ => Err(NetworkError::Rejected),
        }
    }
    fn verify_profile(&mut self, profile: &ProfileId) -> Result<u32, NetworkError> {
        let id = profile
            .as_str()
            .split(':')
            .nth(1)
            .ok_or(NetworkError::InvalidInput)?
            .parse::<u32>()
            .map_err(|_| NetworkError::InvalidInput)?;
        let value = self.control.request(&format!("GET_NETWORK {id} ssid"))?;
        let value = value.trim();
        let ssid = if let Some(quoted) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            decode_ssid(quoted)?
        } else {
            decode_hex(value)?
        };
        if self.owned.contains_key(&id) {
            self.verify_owned(id)?;
        }
        if profile_id(id, &ssid)? != *profile {
            return Err(NetworkError::Rejected);
        }
        Ok(id)
    }
    fn wait_for(&mut self, expected: &NetworkConfiguration) -> Result<(), NetworkError> {
        let deadline = Instant::now() + self.connection_timeout;
        loop {
            if self.observe()? == *expected {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(NetworkError::TimedOut);
            }
            thread::sleep(Duration::from_millis(100));
        }
    }
    fn enabled_profiles(&mut self) -> Result<Vec<(ProfileId, bool)>, NetworkError> {
        let response = self.control.request("LIST_NETWORKS")?;
        if !response.starts_with("network id / ssid / bssid / flags") {
            return Err(NetworkError::Unavailable);
        }
        let mut profiles = Vec::new();
        for line in response.lines().skip(1) {
            if profiles.len() >= 128 {
                return Err(NetworkError::Unavailable);
            }
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.len() != 4 {
                return Err(NetworkError::Unavailable);
            }
            profiles.push((
                profile_id(
                    fields[0].parse().map_err(|_| NetworkError::Unavailable)?,
                    &decode_ssid(fields[1])?,
                )?,
                !fields[3].contains("[DISABLED]"),
            ));
        }
        Ok(profiles)
    }
}
impl<C: WpaControl> NetworkControl for WpaNetworkControl<C> {
    fn check_support(&mut self, intent: &NetworkIntent) -> Result<(), NetworkError> {
        match intent {
            NetworkIntent::ConnectWifi(profile) => {
                let id = self.verify_profile(profile)?;
                if self
                    .control
                    .request(&format!("GET_NETWORK {id} mode"))?
                    .trim()
                    != "0"
                {
                    return Err(NetworkError::Unsupported);
                }
                Ok(())
            }
            NetworkIntent::EnableAccessPoint(profile) => {
                if self.access_point.is_none() {
                    return Err(NetworkError::Unsupported);
                }
                let id = self.verify_profile(profile)?;
                if self
                    .control
                    .request(&format!("GET_NETWORK {id} mode"))?
                    .trim()
                    != "2"
                {
                    return Err(NetworkError::Unsupported);
                }
                Ok(())
            }
            NetworkIntent::DisableAccessPoint => {
                if self.access_point.is_none() {
                    return Err(NetworkError::Unsupported);
                }
                Ok(())
            }
        }
    }
    fn observe(&mut self) -> Result<NetworkConfiguration, NetworkError> {
        let response = self.control.request("STATUS")?;
        let value = |key: &str| response.lines().find_map(|line| line.strip_prefix(key));
        match value("wpa_state=") {
            Some("COMPLETED") => {
                let access_point = match value("mode=") {
                    Some("AP") => true,
                    Some("station") | None => false,
                    Some(_) => return Err(NetworkError::Unsupported),
                };
                let id = value("id=")
                    .ok_or(NetworkError::Unavailable)?
                    .parse::<u32>()
                    .map_err(|_| NetworkError::Unavailable)?;
                let ssid = decode_ssid(value("ssid=").ok_or(NetworkError::Unavailable)?)?;
                let profile = profile_id(id, &ssid)?;
                Ok(if access_point {
                    NetworkConfiguration::AccessPoint(profile)
                } else {
                    NetworkConfiguration::Wifi(profile)
                })
            }
            Some(
                "DISCONNECTED" | "INACTIVE" | "INTERFACE_DISABLED" | "SCANNING" | "ASSOCIATING"
                | "ASSOCIATED" | "4WAY_HANDSHAKE" | "GROUP_HANDSHAKE",
            ) => Ok(NetworkConfiguration::Disconnected),
            _ => Err(NetworkError::Unavailable),
        }
    }
    fn apply(&mut self, intent: &NetworkIntent) -> Result<(), NetworkError> {
        self.check_support(intent)?;
        match intent {
            NetworkIntent::ConnectWifi(profile) => {
                let id = self.verify_profile(profile)?;
                // Leaving access point mode ends the DHCP service and the static address.
                if self.access_point.is_some()
                    && matches!(self.observe(), Ok(NetworkConfiguration::AccessPoint(_)))
                    && let Some(link) = &mut self.access_point
                {
                    link.down()?;
                }
                self.select(id)?;
                self.wait_for(&NetworkConfiguration::Wifi(profile.clone()))
            }
            NetworkIntent::EnableAccessPoint(profile) => {
                let id = self.verify_profile(profile)?;
                self.select(id)?;
                let target = NetworkConfiguration::AccessPoint(profile.clone());
                let result = self.wait_for(&target).and_then(|()| self.link_up());
                if result.is_err()
                    && let Some(link) = &mut self.access_point
                {
                    // A half-started access point must not keep addressing or DHCP alive.
                    let _ = link.down();
                }
                result
            }
            NetworkIntent::DisableAccessPoint => {
                let NetworkConfiguration::AccessPoint(profile) = self.observe()? else {
                    return Ok(());
                };
                if let Some(link) = &mut self.access_point {
                    link.down()?;
                }
                let id = self.verify_profile(&profile)?;
                if !self.owned.contains_key(&id) {
                    self.adopt(id, &profile)?;
                }
                self.remove_owned(id)?;
                self.reenable_foreign_profiles(None)?;
                self.ok("DISCONNECT")?;
                self.wait_for(&NetworkConfiguration::Disconnected)
            }
        }
    }
    fn restore(&mut self, previous: &NetworkConfiguration) -> Result<(), NetworkError> {
        match previous {
            NetworkConfiguration::Wifi(profile) => {
                let id = self.verify_profile(profile)?;
                self.ok(&format!("SELECT_NETWORK {id}"))?;
            }
            NetworkConfiguration::Disconnected => self.ok("DISCONNECT")?,
            NetworkConfiguration::AccessPoint(profile) => {
                let id = self.verify_profile(profile)?;
                self.ok(&format!("SELECT_NETWORK {id}"))?;
                self.wait_for(previous)?;
                return self.link_up();
            }
        }
        if let Some(profiles) = self.enabled_before.clone() {
            for (profile, enabled) in profiles {
                let id = self.verify_profile(&profile)?;
                self.ok(&format!(
                    "{}_NETWORK {id}",
                    if enabled { "ENABLE" } else { "DISABLE" }
                ))?;
            }
        }
        // DISCONNECT after enabling prevents automatic association in the disconnected case.
        if *previous == NetworkConfiguration::Disconnected {
            self.ok("DISCONNECT")?;
        }
        self.wait_for(previous)?;
        self.enabled_before = None;
        Ok(())
    }
}
impl<C: WpaControl> WpaNetworkControl<C> {
    /// SELECT_NETWORK disables every other profile, so remember which ones were enabled.
    fn select(&mut self, id: u32) -> Result<(), NetworkError> {
        // A second connection must not record the profiles the first one disabled.
        if self.enabled_before.is_none() {
            self.enabled_before = Some(self.enabled_profiles()?);
        }
        self.ok(&format!("SELECT_NETWORK {id}"))
    }
    /// A profile this service created before it restarted is recognised by its marker; any
    /// other access point profile belongs to someone else and stays untouched.
    fn adopt(&mut self, id: u32, profile: &ProfileId) -> Result<(), NetworkError> {
        let reply = self.control.request(&format!("GET_NETWORK {id} id_str"))?;
        let marker = reply.trim().trim_matches('"');
        if !marker.starts_with("zorvinkel-") || marker.len() > 128 {
            return Err(NetworkError::Unsupported);
        }
        self.owned.insert(id, profile.clone());
        self.markers.insert(id, marker.into());
        Ok(())
    }
    fn link_up(&mut self) -> Result<(), NetworkError> {
        self.access_point
            .as_mut()
            .ok_or(NetworkError::Unsupported)?
            .up()
    }
}
impl<C: WpaControl> StackControl for WpaNetworkControl<C> {
    fn access_point_supported(&self) -> bool {
        Self::access_point_supported(self)
    }
    fn prepare_wifi(&mut self, ssid: &str, password: &str) -> Result<ProfileId, NetworkError> {
        Self::prepare_wifi(self, ssid, password)
    }
    fn prepare_access_point(
        &mut self,
        ssid: &str,
        password: &str,
    ) -> Result<ProfileId, NetworkError> {
        Self::prepare_access_point(self, ssid, password)
    }
    fn scan(&mut self) -> Result<Vec<String>, NetworkError> {
        Self::scan(self)
    }
    fn discard_inactive_profiles(&mut self) -> Result<(), NetworkError> {
        Self::discard_inactive_profiles(self)
    }
    fn persist_active(&mut self) -> Result<(), NetworkError> {
        Self::persist_active(self)
    }
    fn owned_connection_active(&mut self) -> Result<bool, NetworkError> {
        Self::owned_connection_active(self)
    }
    fn reconnect(&mut self) -> Result<(), NetworkError> {
        Self::reconnect(self)
    }
    fn revert_volatile(&mut self) -> Result<(), NetworkError> {
        Self::revert_volatile(self)
    }
    fn saved_wifi_profile(&mut self, ssid: &str) -> Result<Option<ProfileId>, NetworkError> {
        Self::saved_wifi_profile(self, ssid)
    }
    fn keep_active(&mut self) -> Result<(), NetworkError> {
        Self::keep_active(self)
    }
    fn release_selection(&mut self) -> Result<(), NetworkError> {
        self.reenable_foreign_profiles(None)
    }
}
fn profile_id(id: u32, ssid: &str) -> Result<ProfileId, NetworkError> {
    if ssid.is_empty() || ssid.len() > 32 {
        return Err(NetworkError::Unavailable);
    }
    ProfileId::new(format!("wpa:{id}:{}", hex(ssid.as_bytes())))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn decode_hex(value: &str) -> Result<String, NetworkError> {
    if !value.len().is_multiple_of(2) || !value.is_ascii() {
        return Err(NetworkError::Unavailable);
    }
    let bytes = (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).map_err(|_| NetworkError::Unavailable))
        .collect::<Result<Vec<_>, _>>()?;
    String::from_utf8(bytes).map_err(|_| NetworkError::Unsupported)
}
fn decode_ssid(value: &str) -> Result<String, NetworkError> {
    let mut bytes = Vec::new();
    let mut input = value.bytes();
    while let Some(byte) = input.next() {
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }
        match input.next() {
            Some(b'\\') => bytes.push(b'\\'),
            Some(b'"') => bytes.push(b'"'),
            Some(b'x') => {
                let high = input
                    .next()
                    .and_then(|b| (b as char).to_digit(16))
                    .ok_or(NetworkError::Unavailable)?;
                let low = input
                    .next()
                    .and_then(|b| (b as char).to_digit(16))
                    .ok_or(NetworkError::Unavailable)?;
                bytes.push((high * 16 + low) as u8);
            }
            _ => return Err(NetworkError::Unsupported),
        }
    }
    let ssid = String::from_utf8(bytes).map_err(|_| NetworkError::Unsupported)?;
    if ssid.len() > 32 || ssid.chars().any(char::is_control) {
        return Err(NetworkError::Unsupported);
    }
    Ok(ssid)
}
#[cfg(test)]
mod tests;
