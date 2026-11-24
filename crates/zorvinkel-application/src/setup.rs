//! UI-independent setup commands. Secrets are deliberately not Debug or serialized.
use crate::{
    power::PowerAction,
    services::{ServiceAction, ServiceEntry},
    system::SystemInfo,
};
use zorvinkel_core::{DeviceStatus, DiscoveredDevice, network::RestartPolicy};

#[derive(Clone)]
pub struct AccessPointSettings {
    pub ssid: String,
    pub password: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetupError {
    Disconnected,
    InvalidInput,
    ConnectionRejected,
    Unavailable,
    CredentialsUnavailable,
    IdentityRejected,
    AccessDenied,
    Busy,
    Unsupported,
    RecoveryFailed,
    /// The secured session reached its time or record limit and ended.
    SessionExpired,
    /// A default needs a Wi-Fi connection or the device's own Wi-Fi to be active.
    NoActiveNetwork,
    /// The device does not know this computer and registration is closed: an administrator login
    /// of the device registers it (ADR-037).
    LoginRequired,
    /// The user name or the password of that login is not right.
    LoginRejected,
    /// The login is right, but the account is no administrator of the device.
    NotAdministrator,
    /// The device stopped accepting logins for a while after too many wrong ones.
    TooManyLogins,
}

/// What a connected device can do for its user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Function {
    Wifi,
    AccessPoint,
    RestartOption,
    System,
    Services,
    ServiceActions,
    Power,
    /// Interactive SSH terminal; needs an address of the device or a tunnel over Bluetooth.
    Terminal,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    /// The connected device does not report this function.
    NotOffered,
}
impl SetupSnapshot {
    /// Every function with what the device reported over the current connection. Both
    /// transports carry all operations, so the connection path does not change the answer.
    pub fn functions(&self) -> [(Function, Availability); 8] {
        let state = |offered: bool| {
            if offered {
                Availability::Available
            } else {
                Availability::NotOffered
            }
        };
        [
            (Function::Wifi, state(self.wifi_available)),
            (Function::AccessPoint, state(self.access_point_available)),
            (
                Function::RestartOption,
                state(self.restart_policy.is_some()),
            ),
            (Function::System, state(self.system_info_available)),
            (Function::Services, state(self.services_available)),
            (
                Function::ServiceActions,
                state(self.services_available && self.service_actions_available),
            ),
            (Function::Power, state(self.power_available)),
            (
                Function::Terminal,
                state(
                    self.system_info
                        .as_ref()
                        .is_some_and(|info| info.ssh_address().is_some())
                        || (self.tunnel_available && self.transport == Some(Transport::Ble)),
                ),
            ),
        ]
    }
}

/// The path a connected device is reached over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Ble,
    Ip,
}

pub enum SetupRequest {
    Discover,
    /// Opens a terminal tunnel over the connected Bluetooth device; the stream arrives in
    /// `SetupSnapshot::tunnel` (ADR-031). A refusal is reported in the update, the connection stays.
    OpenTunnel,
    Connect(String),
    /// Like `Connect`, for a computer the device does not know: registers it with the user and
    /// password of an administrator account of the device (ADR-037). The password is used once.
    ConnectWithLogin {
        handle: String,
        user: String,
        password: String,
    },
    Disconnect,
    Refresh,
    Scan,
    ConnectWifi {
        ssid: String,
        password: String,
    },
    /// Stores these credentials on the device and switches its own Wi-Fi on.
    EnableAccessPoint(AccessPointSettings),
    DisableAccessPoint,
    /// Stores new credentials; an active access point restarts with them.
    UpdateAccessPoint(AccessPointSettings),
    RestartLastAccepted,
    RestartCurrentAsDefault,
    /// Switches at once back to the configuration that applies after a restart.
    RestartRevert,
    /// Reads the password for this network on explicit request: the client system's store
    /// first, then the one Zorvinkel remembered after a successful connection.
    ReadSavedCredentials {
        ssid: String,
    },
    /// Keep (true) or discard (false) Wi-Fi connections made from now on after a restart.
    SetWifiPersistence(bool),
    /// Shut down or restart the connected device. The connection ends afterwards.
    PowerDevice(PowerAction),
    /// Starts, stops or restarts the configured service with this name.
    ServiceAction {
        name: String,
        action: ServiceAction,
    },
}

/// Hands one transport stream from the backend thread to the caller. The type of the stream is
/// the adapter's business, so it travels as `Any`; whoever opened it knows what to ask for.
#[derive(Clone, Default)]
pub struct TunnelHandoff(std::sync::Arc<std::sync::Mutex<Option<Box<dyn std::any::Any + Send>>>>);
impl TunnelHandoff {
    pub fn put<T: std::any::Any + Send>(&self, stream: T) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(Box::new(stream));
        }
    }
    /// The stream, once; `None` when none was left or it has another type.
    pub fn take<T: std::any::Any + Send>(&self) -> Option<T> {
        let boxed = self.0.lock().ok()?.take()?;
        boxed.downcast::<T>().ok().map(|stream| *stream)
    }
}

#[derive(Clone, Default)]
pub struct SetupSnapshot {
    pub devices: Vec<DiscoveredDevice>,
    pub status: Option<DeviceStatus>,
    pub networks: Vec<String>,
    pub wifi_available: bool,
    /// The device offers shutdown and restart.
    pub power_available: bool,
    /// The device reports system information; `system_info` holds the latest reading.
    pub system_info_available: bool,
    pub system_info: Option<SystemInfo>,
    /// The device lists configured services; `services` holds the latest reading.
    pub services_available: bool,
    pub services: Vec<ServiceEntry>,
    /// The device performs the actions the services list as allowed.
    pub service_actions_available: bool,
    /// `None` when the device does not offer the choice.
    pub wifi_persistence: Option<bool>,
    /// The device can provide its own Wi-Fi.
    pub access_point_available: bool,
    /// Stored credentials of the device, or editable proposals while it has none.
    pub access_point: Option<AccessPointSettings>,
    /// `access_point` is a proposal that the device has not stored yet.
    pub access_point_proposed: bool,
    pub restart_policy: Option<RestartPolicy>,
    /// The configuration active now is not the one that applies after a restart.
    pub restart_differs: bool,
    /// How the connected device is reached; `None` while disconnected or simulated.
    pub transport: Option<Transport>,
    /// The device offers the terminal tunnel (capability); usable over Bluetooth only.
    pub tunnel_available: bool,
    /// Where a stream opened by `OpenTunnel` is left for the caller.
    pub tunnel: TunnelHandoff,
    /// Time the secured session has left before it ends; `ZERO` once it must be renewed.
    pub session_remaining: Option<std::time::Duration>,
}

pub struct SetupUpdate {
    pub snapshot: SetupSnapshot,
    pub error: Option<SetupError>,
    /// Only set as the answer to `ReadSavedCredentials`; the UI must not keep it.
    pub saved_password: Option<String>,
}

/// Client-side cache of passwords a device accepted. Entries are only written after success.
pub trait WifiPasswordCache {
    fn cached_password(&mut self, ssid: &str) -> Result<Option<String>, SetupError>;
    fn remember(&mut self, ssid: &str, password: &str) -> Result<(), SetupError>;
}

/// Client-side source of Wi-Fi passwords stored by the operating system.
/// Implementations ask the user for approval where the platform requires it.
pub trait ClientCredentialSource {
    fn saved_wifi_password(&mut self, ssid: &str) -> Result<String, SetupError>;
}

/// The runtime executes requests outside the UI thread. Real adapters must authenticate
/// and authorize every device command before calling its application use case.
pub trait SetupBackend: Send + 'static {
    fn handle(&mut self, request: SetupRequest) -> Result<SetupUpdate, SetupError>;
}

impl SetupBackend for Box<dyn SetupBackend> {
    fn handle(&mut self, request: SetupRequest) -> Result<SetupUpdate, SetupError> {
        (**self).handle(request)
    }
}

pub fn validate_wifi(ssid: &str, password: &str) -> Result<(), SetupError> {
    if ssid.is_empty()
        || ssid.len() > 32
        || ssid.chars().any(char::is_control)
        || !(8..=63).contains(&password.len())
        || password.chars().any(char::is_control)
    {
        return Err(SetupError::InvalidInput);
    }
    Ok(())
}

/// Characters of a generated passphrase: no symbols, so it survives QR codes, shells and typing.
const PASSWORD_ALPHABET: &[u8; 62] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
/// WPA2 passphrases end at 63 characters; this length yields about 375 bits, more than the
/// 256-bit key derived from it.
pub const GENERATED_PASSWORD_LENGTH: usize = 63;

/// Proposes `Zorvinkel-<hostname>`, cut to the 32 bytes an SSID may have.
pub fn default_access_point_ssid(hostname: &str) -> String {
    let name = hostname
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>();
    let ssid = if name.is_empty() {
        "Zorvinkel".to_owned()
    } else {
        format!("Zorvinkel-{name}")
    };
    ssid.chars().take(32).collect()
}

/// `fill` supplies cryptographically secure random bytes and reports whether it could.
/// Bytes that would skew the alphabet are dropped, so every character is uniformly
/// distributed. Without randomness there is no password, never a guessable one.
pub fn generate_access_point_password(mut fill: impl FnMut(&mut [u8]) -> bool) -> Option<String> {
    let mut password = String::with_capacity(GENERATED_PASSWORD_LENGTH);
    let mut bytes = [0_u8; 64];
    while password.len() < GENERATED_PASSWORD_LENGTH {
        if !fill(&mut bytes) {
            return None;
        }
        for byte in bytes {
            // 248 = 4 * 62, the largest multiple of the alphabet size within a byte.
            if byte < 248 && password.len() < GENERATED_PASSWORD_LENGTH {
                password.push(char::from(PASSWORD_ALPHABET[usize::from(byte) % 62]));
            }
        }
    }
    Some(password)
}

/// Proposal for a device that has no stored access point credentials yet.
pub fn propose_access_point(
    hostname: &str,
    fill: impl FnMut(&mut [u8]) -> bool,
) -> Option<AccessPointSettings> {
    Some(AccessPointSettings {
        ssid: default_access_point_ssid(hostname),
        password: generate_access_point_password(fill)?,
    })
}

/// WPA2 passphrases are printable ASCII only.
pub fn validate_access_point(ssid: &str, password: &str) -> Result<(), SetupError> {
    if ssid.is_empty()
        || ssid.len() > 32
        || ssid.chars().any(char::is_control)
        || !(8..=63).contains(&password.len())
        || !password.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
    {
        return Err(SetupError::InvalidInput);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn functions_follow_what_the_device_reported() {
        let mut snapshot = SetupSnapshot::default();
        assert!(
            snapshot
                .functions()
                .iter()
                .all(|(_, state)| *state == Availability::NotOffered)
        );
        snapshot.wifi_available = true;
        snapshot.service_actions_available = true;
        snapshot.restart_policy = Some(RestartPolicy::LastAccepted);
        let state = |snapshot: &SetupSnapshot, function| {
            snapshot
                .functions()
                .into_iter()
                .find(|(candidate, _)| *candidate == function)
                .unwrap()
                .1
        };
        assert_eq!(state(&snapshot, Function::Wifi), Availability::Available);
        assert_eq!(
            state(&snapshot, Function::RestartOption),
            Availability::Available
        );
        // Actions on services need the service list as well.
        assert_eq!(
            state(&snapshot, Function::ServiceActions),
            Availability::NotOffered
        );
        snapshot.services_available = true;
        assert_eq!(
            state(&snapshot, Function::ServiceActions),
            Availability::Available
        );
        assert_eq!(state(&snapshot, Function::Power), Availability::NotOffered);
    }
    #[test]
    fn ssid_proposal_follows_the_hostname_and_fits_an_ssid() {
        assert_eq!(default_access_point_ssid("Fotobox"), "Zorvinkel-Fotobox");
        assert_eq!(default_access_point_ssid("a b/ü"), "Zorvinkel-ab");
        assert_eq!(default_access_point_ssid("///"), "Zorvinkel");
        assert_eq!(default_access_point_ssid(&"x".repeat(60)).len(), 32);
    }
    #[test]
    fn generated_password_is_maximal_unbiased_and_valid() {
        let mut counter = 0_u8;
        // Bytes at and above 248 must be skipped, not wrapped.
        let password = generate_access_point_password(|bytes| {
            for byte in bytes {
                *byte = counter;
                counter = counter.wrapping_add(1);
            }
            true
        })
        .unwrap();
        assert_eq!(password.len(), GENERATED_PASSWORD_LENGTH);
        assert!(password.bytes().all(|b| PASSWORD_ALPHABET.contains(&b)));
        assert!(validate_access_point("Zorvinkel", &password).is_ok());
        assert_eq!(&password[..4], "ABCD");
    }
    #[test]
    fn missing_randomness_yields_no_password() {
        assert!(generate_access_point_password(|_| false).is_none());
        assert!(propose_access_point("Box", |_| false).is_none());
    }
    #[test]
    fn access_point_input_is_validated() {
        assert!(validate_access_point("Box", "password-ok").is_ok());
        for (ssid, password) in [
            ("", "password-ok"),
            ("Box", "short"),
            ("Box", "überlanges-passwort"),
            ("Box\n", "password-ok\n"),
            (&"s".repeat(33), "password-ok"),
        ] {
            assert_eq!(
                validate_access_point(ssid, password),
                Err(SetupError::InvalidInput)
            );
        }
    }
}
