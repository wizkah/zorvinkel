//! NetworkManager over the system D-Bus. Only thin translation between `NetworkManagerApi` and
//! the D-Bus interfaces lives here; the rules are in the parent module.
use super::{ConnectionInfo, ConnectionSpec, Link, Mode, NetworkManagerApi, network_status};
use dbus::{
    Path,
    arg::{PropMap, RefArg, Variant},
    blocking::{Connection, Proxy, stdintf::org_freedesktop_dbus::Properties},
};
use std::{
    collections::HashMap,
    thread,
    time::{Duration, Instant},
};
use zorvinkel_application::PortError;
use zorvinkel_core::{NetworkStatus, network::NetworkError};

const SERVICE: &str = "org.freedesktop.NetworkManager";
const ROOT: &str = "/org/freedesktop/NetworkManager";
const SETTINGS: &str = "/org/freedesktop/NetworkManager/Settings";
const ROOT_INTERFACE: &str = "org.freedesktop.NetworkManager";
const SETTINGS_INTERFACE: &str = "org.freedesktop.NetworkManager.Settings";
const CONNECTION_INTERFACE: &str = "org.freedesktop.NetworkManager.Settings.Connection";
const ACTIVE_INTERFACE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const DEVICE_INTERFACE: &str = "org.freedesktop.NetworkManager.Device";
const WIRELESS_INTERFACE: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const ACCESS_POINT_INTERFACE: &str = "org.freedesktop.NetworkManager.AccessPoint";

const DEVICE_TYPE_WIFI: u32 = 2;
const ACTIVE_STATE_ACTIVATED: u32 = 2;
/// `NM_WIFI_DEVICE_CAP_AP`.
const CAPABILITY_ACCESS_POINT: u32 = 0x40;
/// `NM_SETTINGS_ADD_CONNECTION2_FLAG_IN_MEMORY | ..._BLOCK_AUTOCONNECT`.
const ADD_IN_MEMORY_BLOCKED: u32 = 0x2 | 0x20;
/// `NM_SETTINGS_UPDATE2_FLAG_TO_DISK`.
const UPDATE_TO_DISK: u32 = 0x1;

const CALL_TIMEOUT: Duration = Duration::from_secs(5);
const SCAN_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_CONNECTIONS: usize = 512;
const MAX_WIFI_CONNECTIONS: usize = 128;
const MAX_ACCESS_POINTS: usize = 256;
/// The device's own subnet, as for the wpa_supplicant fallback (ADR-017).
const ACCESS_POINT_ADDRESS: &str = "192.168.4.1";
const ACCESS_POINT_CHANNEL: u32 = 6;

type Settings = HashMap<String, PropMap>;

pub struct NetworkManagerBus {
    connection: Connection,
    interface: String,
    device: Path<'static>,
}
impl NetworkManagerBus {
    /// Fails when NetworkManager does not answer or the interface is not a Wi-Fi device.
    pub fn open(interface: &str) -> Result<Self, NetworkError> {
        let connection = Connection::new_system().map_err(map_error)?;
        let (device,): (Path<'static>,) = connection
            .with_proxy(SERVICE, ROOT, CALL_TIMEOUT)
            .method_call(ROOT_INTERFACE, "GetDeviceByIpIface", (interface,))
            .map_err(map_error)?;
        let bus = Self {
            connection,
            interface: interface.into(),
            device,
        };
        let kind: u32 = bus
            .device_proxy()
            .get(DEVICE_INTERFACE, "DeviceType")
            .map_err(map_error)?;
        if kind != DEVICE_TYPE_WIFI {
            return Err(NetworkError::Unsupported);
        }
        Ok(bus)
    }
    fn proxy(&self, path: impl Into<Path<'static>>) -> Proxy<'_, &Connection> {
        self.connection.with_proxy(SERVICE, path, CALL_TIMEOUT)
    }
    fn device_proxy(&self) -> Proxy<'_, &Connection> {
        self.proxy(self.device.clone())
    }
    fn connection_by_uuid(&self, uuid: &str) -> Result<Path<'static>, NetworkError> {
        let (path,): (Path<'static>,) = self
            .proxy(SETTINGS)
            .method_call(SETTINGS_INTERFACE, "GetConnectionByUuid", (uuid,))
            .map_err(map_error)?;
        Ok(path)
    }
    fn settings(&self, path: Path<'static>) -> Result<Settings, NetworkError> {
        let (settings,): (Settings,) = self
            .proxy(path)
            .method_call(CONNECTION_INTERFACE, "GetSettings", ())
            .map_err(map_error)?;
        Ok(settings)
    }
    /// `None` for profiles that are not Wi-Fi or belong to another interface. Secrets are not
    /// part of the answer.
    fn describe(&self, path: Path<'static>) -> Result<Option<ConnectionInfo>, NetworkError> {
        let saved = !self
            .proxy(path.clone())
            .get::<bool>(CONNECTION_INTERFACE, "Unsaved")
            .map_err(map_error)?;
        let settings = self.settings(path)?;
        Ok(describe_settings(&settings, &self.interface, saved))
    }
    fn active_connection(&self) -> Result<Option<Path<'static>>, NetworkError> {
        let path: Path<'static> = self
            .device_proxy()
            .get(DEVICE_INTERFACE, "ActiveConnection")
            .map_err(map_error)?;
        Ok((&*path != "/").then_some(path))
    }
}
impl NetworkManagerApi for NetworkManagerBus {
    fn access_point_capable(&mut self) -> Result<bool, NetworkError> {
        let capabilities: u32 = self
            .device_proxy()
            .get(WIRELESS_INTERFACE, "WirelessCapabilities")
            .map_err(map_error)?;
        Ok(capabilities & CAPABILITY_ACCESS_POINT != 0)
    }
    fn link(&mut self) -> Result<Link, NetworkError> {
        let Some(active) = self.active_connection()? else {
            return Ok(Link::Down);
        };
        let active = self.proxy(active);
        let state: u32 = active.get(ACTIVE_INTERFACE, "State").map_err(map_error)?;
        if state != ACTIVE_STATE_ACTIVATED {
            return Ok(Link::Down);
        }
        let uuid: String = active.get(ACTIVE_INTERFACE, "Uuid").map_err(map_error)?;
        let path = self.connection_by_uuid(&uuid)?;
        let info = self.describe(path)?.ok_or(NetworkError::Unsupported)?;
        Ok(Link::Activated {
            uuid,
            ssid: info.ssid,
            mode: info.mode,
        })
    }
    fn connections(&mut self) -> Result<Vec<ConnectionInfo>, NetworkError> {
        let (paths,): (Vec<Path<'static>>,) = self
            .proxy(SETTINGS)
            .method_call(SETTINGS_INTERFACE, "ListConnections", ())
            .map_err(map_error)?;
        if paths.len() > MAX_CONNECTIONS {
            return Err(NetworkError::Unavailable);
        }
        let mut connections = Vec::new();
        for path in paths {
            // A profile can vanish between listing and reading.
            if let Ok(Some(info)) = self.describe(path) {
                connections.push(info);
            }
            if connections.len() > MAX_WIFI_CONNECTIONS {
                return Err(NetworkError::Unavailable);
            }
        }
        Ok(connections)
    }
    fn add_connection(&mut self, spec: &ConnectionSpec) -> Result<String, NetworkError> {
        let uuid = random_uuid()?;
        let settings = build_settings(spec, &uuid, &self.interface);
        let (_path, _result): (Path<'static>, PropMap) = self
            .proxy(SETTINGS)
            .method_call(
                SETTINGS_INTERFACE,
                "AddConnection2",
                (settings, ADD_IN_MEMORY_BLOCKED, PropMap::new()),
            )
            .map_err(map_error)?;
        Ok(uuid)
    }
    fn save_connection(&mut self, uuid: &str) -> Result<(), NetworkError> {
        let path = self.connection_by_uuid(uuid)?;
        // Empty settings keep the profile as it is; only the flag changes where it is stored.
        let _: (PropMap,) = self
            .proxy(path)
            .method_call(
                CONNECTION_INTERFACE,
                "Update2",
                (Settings::new(), UPDATE_TO_DISK, PropMap::new()),
            )
            .map_err(map_error)?;
        Ok(())
    }
    fn delete_connection(&mut self, uuid: &str) -> Result<(), NetworkError> {
        let path = self.connection_by_uuid(uuid)?;
        self.proxy(path)
            .method_call::<(), _, _, _>(CONNECTION_INTERFACE, "Delete", ())
            .map_err(map_error)
    }
    fn activate(&mut self, uuid: &str) -> Result<(), NetworkError> {
        let path = self.connection_by_uuid(uuid)?;
        let _: (Path<'static>,) = self
            .proxy(ROOT)
            .method_call(
                ROOT_INTERFACE,
                "ActivateConnection",
                (path, self.device.clone(), Path::from("/")),
            )
            .map_err(map_error)?;
        Ok(())
    }
    fn disconnect(&mut self) -> Result<(), NetworkError> {
        match self
            .device_proxy()
            .method_call::<(), _, _, _>(DEVICE_INTERFACE, "Disconnect", ())
        {
            Err(error)
                if error
                    .name()
                    .is_some_and(|name| name.ends_with(".NotActive")) =>
            {
                Ok(())
            }
            result => result.map_err(map_error),
        }
    }
    fn set_autoconnect(&mut self, enabled: bool) -> Result<(), NetworkError> {
        self.device_proxy()
            .set(DEVICE_INTERFACE, "Autoconnect", enabled)
            .map_err(map_error)
    }
    fn scan(&mut self) -> Result<Vec<String>, NetworkError> {
        let before: i64 = self
            .device_proxy()
            .get(WIRELESS_INTERFACE, "LastScan")
            .map_err(map_error)?;
        match self.device_proxy().method_call::<(), _, _, _>(
            WIRELESS_INTERFACE,
            "RequestScan",
            (PropMap::new(),),
        ) {
            Ok(()) => self.wait_for_scan(before)?,
            // NetworkManager refuses a scan right after another one; its results are fresh.
            Err(error)
                if error
                    .name()
                    .is_some_and(|name| name.ends_with(".NotAllowed")) => {}
            Err(error) => return Err(map_error(error)),
        }
        self.access_point_names()
    }
}
impl NetworkManagerBus {
    fn wait_for_scan(&self, before: i64) -> Result<(), NetworkError> {
        let deadline = Instant::now() + SCAN_TIMEOUT;
        loop {
            let now: i64 = self
                .device_proxy()
                .get(WIRELESS_INTERFACE, "LastScan")
                .map_err(map_error)?;
            if now != before {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(NetworkError::TimedOut);
            }
            thread::sleep(Duration::from_millis(250));
        }
    }
    fn access_point_names(&self) -> Result<Vec<String>, NetworkError> {
        let (paths,): (Vec<Path<'static>>,) = self
            .device_proxy()
            .method_call(WIRELESS_INTERFACE, "GetAllAccessPoints", ())
            .map_err(map_error)?;
        let mut names = Vec::new();
        for path in paths.into_iter().take(MAX_ACCESS_POINTS) {
            let Ok(bytes) = self
                .proxy(path)
                .get::<Vec<u8>>(ACCESS_POINT_INTERFACE, "Ssid")
            else {
                continue;
            };
            if let Some(ssid) = visible_ssid(bytes)
                && !names.contains(&ssid)
            {
                names.push(ssid);
            }
        }
        Ok(names)
    }
}

/// Reads the status for the client. The D-Bus connection is opened on demand and dropped after
/// an error, so a restarted NetworkManager is picked up again.
pub struct NetworkManagerStatus {
    interface: String,
    bus: Option<NetworkManagerBus>,
}
impl NetworkManagerStatus {
    pub fn new(interface: &str) -> Self {
        Self {
            interface: interface.into(),
            bus: None,
        }
    }
    /// Blocks for at most a few D-Bus timeouts; call it from a blocking context.
    pub fn read(&mut self) -> Result<NetworkStatus, PortError> {
        let mut bus = match self.bus.take() {
            Some(bus) => bus,
            None => NetworkManagerBus::open(&self.interface).map_err(|_| PortError::Unavailable)?,
        };
        let status = network_status(&mut bus);
        if status.is_ok() {
            self.bus = Some(bus);
        }
        status
    }
}

/// Hidden and non-text names are not offered for selection.
fn visible_ssid(bytes: Vec<u8>) -> Option<String> {
    let ssid = String::from_utf8(bytes).ok()?;
    (!ssid.is_empty() && ssid.len() <= 32 && !ssid.chars().any(char::is_control)).then_some(ssid)
}

fn describe_settings(settings: &Settings, interface: &str, saved: bool) -> Option<ConnectionInfo> {
    let connection = settings.get("connection")?;
    let text = |map: &PropMap, key: &str| {
        map.get(key)
            .and_then(|value| value.0.as_str())
            .map(str::to_owned)
    };
    if text(connection, "type")? != "802-11-wireless" {
        return None;
    }
    if text(connection, "interface-name").is_some_and(|name| name != interface) {
        return None;
    }
    let wireless = settings.get("802-11-wireless")?;
    let ssid = String::from_utf8(
        wireless
            .get("ssid")?
            .0
            .as_iter()?
            .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
            .collect::<Option<Vec<_>>>()?,
    )
    .ok()?;
    let mode = match text(wireless, "mode").as_deref() {
        Some("ap") => Mode::AccessPoint,
        Some("infrastructure") | None => Mode::Station,
        Some(_) => return None,
    };
    Some(ConnectionInfo {
        uuid: text(connection, "uuid")?,
        name: text(connection, "id")?,
        ssid,
        mode,
        saved,
    })
}

fn variant(value: impl RefArg + 'static) -> Variant<Box<dyn RefArg>> {
    Variant(Box::new(value))
}
fn section<const N: usize>(entries: [(&str, Variant<Box<dyn RefArg>>); N]) -> PropMap {
    entries
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect()
}
/// WPA2-Personal with CCMP only, as for the wpa_supplicant fallback. A profile never connects
/// on its own until it is activated explicitly; station profiles join again once saved.
fn build_settings(spec: &ConnectionSpec, uuid: &str, interface: &str) -> Settings {
    let access_point = spec.mode == Mode::AccessPoint;
    let mut wireless = section([
        ("ssid", variant(spec.ssid.as_bytes().to_vec())),
        (
            "mode",
            variant(if access_point { "ap" } else { "infrastructure" }.to_owned()),
        ),
    ]);
    let mut security = section([
        ("key-mgmt", variant("wpa-psk".to_owned())),
        ("psk", variant(spec.password.to_owned())),
        ("proto", variant(vec!["rsn".to_owned()])),
    ]);
    let (ipv4, ipv6) = if access_point {
        wireless.extend(section([
            ("band", variant("bg".to_owned())),
            ("channel", variant(ACCESS_POINT_CHANNEL)),
        ]));
        security.extend(section([
            ("pairwise", variant(vec!["ccmp".to_owned()])),
            ("group", variant(vec!["ccmp".to_owned()])),
        ]));
        let address = section([
            ("address", variant(ACCESS_POINT_ADDRESS.to_owned())),
            ("prefix", variant(24_u32)),
        ]);
        (
            section([
                ("method", variant("shared".to_owned())),
                ("address-data", variant(vec![address])),
            ]),
            section([("method", variant("ignore".to_owned()))]),
        )
    } else {
        wireless.extend(section([("hidden", variant(true))]));
        (
            section([("method", variant("auto".to_owned()))]),
            section([("method", variant("auto".to_owned()))]),
        )
    };
    HashMap::from([
        (
            "connection".to_owned(),
            section([
                ("id", variant(spec.name.to_owned())),
                ("uuid", variant(uuid.to_owned())),
                ("type", variant("802-11-wireless".to_owned())),
                ("interface-name", variant(interface.to_owned())),
                ("autoconnect", variant(!access_point)),
            ]),
        ),
        ("802-11-wireless".to_owned(), wireless),
        ("802-11-wireless-security".to_owned(), security),
        ("ipv4".to_owned(), ipv4),
        ("ipv6".to_owned(), ipv6),
    ])
}

fn random_uuid() -> Result<String, NetworkError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| NetworkError::Unavailable)?;
    bytes[6] = bytes[6] & 0x0f | 0x40;
    bytes[8] = bytes[8] & 0x3f | 0x80;
    let hex = bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

fn map_error(error: dbus::Error) -> NetworkError {
    match error.name() {
        Some("org.freedesktop.DBus.Error.NoReply" | "org.freedesktop.DBus.Error.Timeout") => {
            NetworkError::TimedOut
        }
        Some(name) if name.ends_with(".PermissionDenied") => NetworkError::Rejected,
        _ => NetworkError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(mode: Mode) -> ConnectionSpec<'static> {
        ConnectionSpec {
            name: "zorvinkel-1-1",
            ssid: "Event",
            password: "test-passphrase",
            mode,
        }
    }
    #[test]
    fn station_profile_joins_on_its_own_and_probes_hidden_networks() {
        let settings = build_settings(&spec(Mode::Station), "u-1", "wlan0");
        let text = |section: &str, key: &str| {
            settings[section][key]
                .0
                .as_str()
                .map(str::to_owned)
                .unwrap_or_default()
        };
        assert_eq!(text("connection", "interface-name"), "wlan0");
        assert_eq!(text("802-11-wireless", "mode"), "infrastructure");
        assert_eq!(text("802-11-wireless-security", "psk"), "test-passphrase");
        assert_eq!(text("ipv4", "method"), "auto");
        assert_eq!(settings["connection"]["autoconnect"].0.as_u64(), Some(1));
        assert_eq!(settings["802-11-wireless"]["hidden"].0.as_u64(), Some(1));
    }
    #[test]
    fn access_point_profile_is_ccmp_only_on_the_fixed_subnet_and_never_autostarts() {
        let settings = build_settings(&spec(Mode::AccessPoint), "u-1", "wlan0");
        let text = |section: &str, key: &str| {
            settings[section][key]
                .0
                .as_str()
                .map(str::to_owned)
                .unwrap_or_default()
        };
        assert_eq!(text("802-11-wireless", "mode"), "ap");
        assert_eq!(text("ipv4", "method"), "shared");
        assert_eq!(settings["connection"]["autoconnect"].0.as_u64(), Some(0));
        assert_eq!(
            settings["802-11-wireless"]["channel"].0.as_u64(),
            Some(u64::from(ACCESS_POINT_CHANNEL))
        );
        assert!(settings["802-11-wireless-security"].contains_key("pairwise"));
        assert!(!settings["802-11-wireless"].contains_key("hidden"));
    }
    #[test]
    fn settings_of_other_interfaces_and_kinds_are_not_described() {
        let settings = build_settings(&spec(Mode::Station), "u-1", "wlan0");
        let info = describe_settings(&settings, "wlan0", false).unwrap();
        assert_eq!(
            (
                info.uuid.as_str(),
                info.ssid.as_str(),
                info.mode,
                info.saved
            ),
            ("u-1", "Event", Mode::Station, false)
        );
        assert!(describe_settings(&settings, "wlan1", true).is_none());
        let mut ethernet = build_settings(&spec(Mode::Station), "u-1", "wlan0");
        ethernet
            .get_mut("connection")
            .unwrap()
            .insert("type".into(), variant("802-3-ethernet".to_owned()));
        assert!(describe_settings(&ethernet, "wlan0", true).is_none());
    }
    #[test]
    fn identifiers_are_random_version_four_uuids() {
        let first = random_uuid().unwrap();
        assert_ne!(first, random_uuid().unwrap());
        assert_eq!(first.len(), 36);
        assert_eq!(first.as_bytes()[14], b'4');
    }
    #[test]
    fn only_readable_names_are_offered() {
        assert_eq!(visible_ssid(b"Home".to_vec()).as_deref(), Some("Home"));
        assert!(visible_ssid(Vec::new()).is_none());
        assert!(visible_ssid(vec![0xff, 0xfe]).is_none());
        assert!(visible_ssid(b"a\nb".to_vec()).is_none());
        assert!(visible_ssid(vec![b'a'; 33]).is_none());
    }
}
