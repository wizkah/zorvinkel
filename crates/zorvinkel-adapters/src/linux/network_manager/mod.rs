//! NetworkManager adapter (ADR-015, ADR-018). NetworkManager is reached only through
//! `NetworkManagerApi`, so the rules below are testable without D-Bus.
//!
//! Profiles created by this service are named `zorvinkel-<id>` and kept in memory only until
//! the caller persists the active one. Foreign profiles are never changed or deleted.
#[cfg(all(target_os = "linux", feature = "network-manager"))]
mod bus;
#[cfg(test)]
mod tests;

#[cfg(all(target_os = "linux", feature = "network-manager"))]
pub use bus::{NetworkManagerBus, NetworkManagerStatus};

use super::stack::StackControl;
use std::{
    collections::BTreeMap,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use zorvinkel_application::{
    PortError,
    network::NetworkControl,
    setup::{validate_access_point, validate_wifi},
};
use zorvinkel_core::{NetworkStatus, network::*};

const NAME_PREFIX: &str = "zorvinkel-";
const MAX_OWNED: usize = 16;
/// Activation includes DHCP, so it takes longer than joining with wpa_supplicant alone.
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Station,
    AccessPoint,
}

/// A new Wi-Fi profile. The password only travels through the D-Bus call, never through
/// process arguments or files.
pub struct ConnectionSpec<'a> {
    pub name: &'a str,
    pub ssid: &'a str,
    pub password: &'a str,
    pub mode: Mode,
}

/// A Wi-Fi profile known to NetworkManager. `saved` is false for in-memory profiles.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionInfo {
    pub uuid: String,
    pub name: String,
    pub ssid: String,
    pub mode: Mode,
    pub saved: bool,
}

/// The fully activated connection on the interface, or nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Activated {
        uuid: String,
        ssid: String,
        mode: Mode,
    },
    Down,
}

/// Every call must be bounded in time and size.
pub trait NetworkManagerApi {
    fn access_point_capable(&mut self) -> Result<bool, NetworkError>;
    fn link(&mut self) -> Result<Link, NetworkError>;
    /// Wi-Fi profiles for the interface, bounded.
    fn connections(&mut self) -> Result<Vec<ConnectionInfo>, NetworkError>;
    /// Adds an in-memory profile that does not connect on its own; returns its UUID.
    fn add_connection(&mut self, spec: &ConnectionSpec) -> Result<String, NetworkError>;
    /// Writes an in-memory profile to disk.
    fn save_connection(&mut self, uuid: &str) -> Result<(), NetworkError>;
    fn delete_connection(&mut self, uuid: &str) -> Result<(), NetworkError>;
    /// Starts the activation; completion is observed through `link`.
    fn activate(&mut self, uuid: &str) -> Result<(), NetworkError>;
    /// Deactivates the interface and keeps NetworkManager from reconnecting by itself.
    fn disconnect(&mut self) -> Result<(), NetworkError>;
    fn set_autoconnect(&mut self, enabled: bool) -> Result<(), NetworkError>;
    /// Requests a scan, waits for it and returns the SSIDs of the fresh results.
    fn scan(&mut self) -> Result<Vec<String>, NetworkError>;
}

pub struct NmNetworkControl<A> {
    api: A,
    access_point: bool,
    /// UUID to profile name: the name proves that a UUID is still the profile created here.
    owned: BTreeMap<String, String>,
    connection_timeout: Duration,
}
impl<A: NetworkManagerApi> NmNetworkControl<A> {
    pub fn new(mut api: A) -> Self {
        // Without a capability answer the access point is simply not offered.
        let access_point = api.access_point_capable().unwrap_or(false);
        Self {
            api,
            access_point,
            owned: BTreeMap::new(),
            connection_timeout: CONNECTION_TIMEOUT,
        }
    }

    fn prepare(
        &mut self,
        ssid: &str,
        password: &str,
        mode: Mode,
    ) -> Result<ProfileId, NetworkError> {
        if self.owned.len() >= MAX_OWNED {
            return Err(NetworkError::Unavailable);
        }
        self.remove_stale()?;
        let name = format!(
            "{NAME_PREFIX}{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| NetworkError::Unavailable)?
                .as_nanos()
        );
        let uuid = self.api.add_connection(&ConnectionSpec {
            name: &name,
            ssid,
            password,
            mode,
        })?;
        let profile = profile_id(&uuid, ssid)?;
        self.owned.insert(uuid, name);
        Ok(profile)
    }
    fn active_uuid(&mut self) -> Result<Option<String>, NetworkError> {
        Ok(match self.api.link()? {
            Link::Activated { uuid, .. } => Some(uuid),
            Link::Down => None,
        })
    }
    /// In-memory profiles of an earlier run of the service; NetworkManager keeps them alive.
    fn remove_stale(&mut self) -> Result<(), NetworkError> {
        let active = self.active_uuid()?;
        for info in self.api.connections()? {
            if info.name.starts_with(NAME_PREFIX)
                && !info.saved
                && !self.owned.contains_key(&info.uuid)
                && active.as_deref() != Some(info.uuid.as_str())
            {
                self.api.delete_connection(&info.uuid)?;
            }
        }
        Ok(())
    }
    fn verify_profile(&mut self, profile: &ProfileId) -> Result<ConnectionInfo, NetworkError> {
        let uuid = profile_uuid(profile)?;
        let info = self
            .api
            .connections()?
            .into_iter()
            .find(|info| info.uuid == uuid)
            .ok_or(NetworkError::Rejected)?;
        if profile_id(&info.uuid, &info.ssid)? != *profile {
            return Err(NetworkError::Rejected);
        }
        if let Some(name) = self.owned.get(&info.uuid)
            && *name != info.name
        {
            return Err(NetworkError::Rejected);
        }
        Ok(info)
    }
    /// A profile this service created before it restarted is recognised by its name; any other
    /// access point profile belongs to someone else and stays untouched.
    fn adopt(&mut self, info: &ConnectionInfo) -> Result<(), NetworkError> {
        if !info.name.starts_with(NAME_PREFIX) || info.name.len() > 128 {
            return Err(NetworkError::Unsupported);
        }
        self.owned.insert(info.uuid.clone(), info.name.clone());
        Ok(())
    }
    fn remove_owned(&mut self, uuid: &str) -> Result<(), NetworkError> {
        let name = self.owned.get(uuid).ok_or(NetworkError::InvalidInput)?;
        // The profile may be gone already, or its UUID reused by someone else.
        if let Some(info) = self
            .api
            .connections()?
            .into_iter()
            .find(|info| info.uuid == uuid)
        {
            if info.name != *name {
                return Err(NetworkError::Rejected);
            }
            self.api.delete_connection(uuid)?;
        }
        self.owned.remove(uuid);
        Ok(())
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
            thread::sleep(Duration::from_millis(250));
        }
    }
    fn activate(&mut self, profile: &ProfileId, mode: Mode) -> Result<(), NetworkError> {
        let info = self.verify_profile(profile)?;
        if info.mode != mode {
            return Err(NetworkError::Unsupported);
        }
        self.api.activate(&info.uuid)
    }
}
impl<A: NetworkManagerApi> NetworkControl for NmNetworkControl<A> {
    fn check_support(&mut self, intent: &NetworkIntent) -> Result<(), NetworkError> {
        match intent {
            NetworkIntent::ConnectWifi(profile) => {
                if self.verify_profile(profile)?.mode != Mode::Station {
                    return Err(NetworkError::Unsupported);
                }
            }
            NetworkIntent::EnableAccessPoint(profile) => {
                if !self.access_point || self.verify_profile(profile)?.mode != Mode::AccessPoint {
                    return Err(NetworkError::Unsupported);
                }
            }
            NetworkIntent::DisableAccessPoint => {
                if !self.access_point {
                    return Err(NetworkError::Unsupported);
                }
            }
        }
        Ok(())
    }
    fn observe(&mut self) -> Result<NetworkConfiguration, NetworkError> {
        Ok(match self.api.link()? {
            Link::Down => NetworkConfiguration::Disconnected,
            Link::Activated { uuid, ssid, mode } => {
                let profile = profile_id(&uuid, &ssid)?;
                match mode {
                    Mode::Station => NetworkConfiguration::Wifi(profile),
                    Mode::AccessPoint => NetworkConfiguration::AccessPoint(profile),
                }
            }
        })
    }
    fn apply(&mut self, intent: &NetworkIntent) -> Result<(), NetworkError> {
        self.check_support(intent)?;
        match intent {
            NetworkIntent::ConnectWifi(profile) => {
                self.activate(profile, Mode::Station)?;
                self.wait_for(&NetworkConfiguration::Wifi(profile.clone()))
            }
            NetworkIntent::EnableAccessPoint(profile) => {
                self.activate(profile, Mode::AccessPoint)?;
                self.wait_for(&NetworkConfiguration::AccessPoint(profile.clone()))
            }
            NetworkIntent::DisableAccessPoint => {
                let NetworkConfiguration::AccessPoint(profile) = self.observe()? else {
                    return Ok(());
                };
                let info = self.verify_profile(&profile)?;
                self.adopt(&info)?;
                // Disconnecting first keeps NetworkManager from joining another network before
                // the caller asks for it (`reconnect`).
                self.api.disconnect()?;
                self.remove_owned(&info.uuid)?;
                self.wait_for(&NetworkConfiguration::Disconnected)
            }
        }
    }
    fn restore(&mut self, previous: &NetworkConfiguration) -> Result<(), NetworkError> {
        match previous {
            NetworkConfiguration::Wifi(profile) => {
                self.api.set_autoconnect(true)?;
                self.activate(profile, Mode::Station)?;
            }
            NetworkConfiguration::AccessPoint(profile) => {
                self.activate(profile, Mode::AccessPoint)?;
            }
            NetworkConfiguration::Disconnected => self.api.disconnect()?,
        }
        self.wait_for(previous)
    }
}
impl<A: NetworkManagerApi> StackControl for NmNetworkControl<A> {
    fn access_point_supported(&self) -> bool {
        self.access_point
    }
    fn prepare_wifi(&mut self, ssid: &str, password: &str) -> Result<ProfileId, NetworkError> {
        validate_wifi(ssid, password).map_err(|_| NetworkError::InvalidInput)?;
        self.prepare(ssid, password, Mode::Station)
    }
    fn prepare_access_point(
        &mut self,
        ssid: &str,
        password: &str,
    ) -> Result<ProfileId, NetworkError> {
        validate_access_point(ssid, password).map_err(|_| NetworkError::InvalidInput)?;
        if !self.access_point {
            return Err(NetworkError::Unsupported);
        }
        self.prepare(ssid, password, Mode::AccessPoint)
    }
    fn scan(&mut self) -> Result<Vec<String>, NetworkError> {
        self.api.scan()
    }
    fn discard_inactive_profiles(&mut self) -> Result<(), NetworkError> {
        let active = self.active_uuid()?;
        let inactive = self
            .owned
            .keys()
            .filter(|uuid| active.as_deref() != Some(uuid.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        for uuid in inactive {
            self.remove_owned(&uuid)?;
        }
        Ok(())
    }
    /// The saved profile becomes an ordinary one: it is no longer owned, so a later cleanup
    /// never deletes it from disk. Older saved profiles of this service for the same network
    /// are replaced; other people's profiles stay.
    fn persist_active(&mut self) -> Result<(), NetworkError> {
        let Link::Activated {
            uuid,
            ssid,
            mode: Mode::Station,
        } = self.api.link()?
        else {
            return Err(NetworkError::Rejected);
        };
        if !self.owned.contains_key(&uuid) {
            return Err(NetworkError::Rejected);
        }
        self.discard_inactive_profiles()?;
        self.api.save_connection(&uuid)?;
        self.owned.remove(&uuid);
        for info in self.api.connections()? {
            if info.saved
                && info.mode == Mode::Station
                && info.ssid == ssid
                && info.uuid != uuid
                && info.name.starts_with(NAME_PREFIX)
            {
                self.api.delete_connection(&info.uuid)?;
            }
        }
        if self.active_uuid()?.as_deref() != Some(uuid.as_str()) {
            return Err(NetworkError::Rejected);
        }
        Ok(())
    }
    fn owned_connection_active(&mut self) -> Result<bool, NetworkError> {
        Ok(self
            .active_uuid()?
            .is_some_and(|uuid| self.owned.contains_key(&uuid)))
    }
    fn saved_wifi_profile(&mut self, ssid: &str) -> Result<Option<ProfileId>, NetworkError> {
        self.api
            .connections()?
            .into_iter()
            .find(|info| info.saved && info.mode == Mode::Station && info.ssid == ssid)
            .map(|info| profile_id(&info.uuid, &info.ssid))
            .transpose()
    }
    fn keep_active(&mut self) -> Result<(), NetworkError> {
        match self.api.link()? {
            Link::Activated {
                uuid,
                mode: Mode::Station,
                ..
            } if self.owned.contains_key(&uuid) => self.persist_active(),
            Link::Activated {
                mode: Mode::Station,
                ..
            } => Ok(()),
            _ => Err(NetworkError::Rejected),
        }
    }
    fn reconnect(&mut self) -> Result<(), NetworkError> {
        self.api.set_autoconnect(true)
    }
    /// Deleting the in-memory profiles ends their connection; NetworkManager then picks a saved
    /// network again. It needs no memory of the change, so it also works after a restart.
    fn revert_volatile(&mut self) -> Result<(), NetworkError> {
        for info in self.api.connections()? {
            if info.name.starts_with(NAME_PREFIX) && !info.saved {
                self.api.delete_connection(&info.uuid)?;
            }
        }
        self.owned.clear();
        self.api.set_autoconnect(true)
    }
}

/// The status as the client sees it, read from NetworkManager.
pub fn network_status(api: &mut impl NetworkManagerApi) -> Result<NetworkStatus, PortError> {
    match api.link().map_err(|_| PortError::Unavailable)? {
        Link::Down => Ok(NetworkStatus::Disconnected),
        Link::Activated { ssid, mode, .. } => {
            if ssid.is_empty() || ssid.len() > 32 || ssid.chars().any(char::is_control) {
                return Err(PortError::Unavailable);
            }
            Ok(match mode {
                Mode::Station => NetworkStatus::WifiConnected { ssid },
                Mode::AccessPoint => NetworkStatus::AccessPointActive { ssid },
            })
        }
    }
}

fn profile_id(uuid: &str, ssid: &str) -> Result<ProfileId, NetworkError> {
    if ssid.is_empty() || ssid.len() > 32 || !valid_uuid(uuid) {
        return Err(NetworkError::Unavailable);
    }
    let hex = ssid
        .bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    ProfileId::new(format!("nm:{uuid}:{hex}"))
}
fn profile_uuid(profile: &ProfileId) -> Result<&str, NetworkError> {
    match profile.as_str().split(':').collect::<Vec<_>>()[..] {
        ["nm", uuid, _] if valid_uuid(uuid) => Ok(uuid),
        _ => Err(NetworkError::InvalidInput),
    }
}
fn valid_uuid(uuid: &str) -> bool {
    !uuid.is_empty()
        && uuid.len() <= 64
        && uuid
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}
