//! Asynchronous, authorized network setup. Credentials must never enter durable state.
use zorvinkel_core::registry::ClientKey;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WifiTaskState {
    Pending,
    Networks(Vec<String>),
    Connected,
    Rejected,
    Unavailable,
    RecoveryFailed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WifiError {
    Busy,
    Conflict,
    Unavailable,
    Unauthorized,
    Unsupported,
}
/// What the device applies after a restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartState {
    LastAccepted,
    DefaultWifi,
    DefaultAccessPoint,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartChange {
    LastAccepted,
    /// The configuration that is active now becomes the default.
    CurrentAsDefault,
}
pub trait WifiSetupControl {
    fn scan(&mut self, actor: &ClientKey, id: u64) -> Result<(), WifiError>;
    fn connect(
        &mut self,
        actor: &ClientKey,
        id: u64,
        ssid: &str,
        password: &str,
    ) -> Result<(), WifiError>;
    fn poll(&mut self, actor: &ClientKey, id: u64) -> Result<WifiTaskState, WifiError>;
    /// Whether the device can provide its own Wi-Fi.
    fn access_point_supported(&self) -> bool {
        false
    }
    /// Stored credentials, or `None` while the device has none.
    fn access_point(
        &mut self,
        _actor: &ClientKey,
    ) -> Result<Option<crate::setup::AccessPointSettings>, WifiError> {
        Err(WifiError::Unsupported)
    }
    /// Stores credentials; they apply the next time the access point starts.
    fn set_access_point(
        &mut self,
        _actor: &ClientKey,
        _ssid: &str,
        _password: &str,
    ) -> Result<(), WifiError> {
        Err(WifiError::Unsupported)
    }
    /// Starts switching the access point on or off; progress is polled like other tasks.
    fn switch_access_point(
        &mut self,
        _actor: &ClientKey,
        _id: u64,
        _enabled: bool,
    ) -> Result<(), WifiError> {
        Err(WifiError::Unsupported)
    }
    /// An authenticated client is talking to the device; it confirms a pending Wi-Fi change.
    fn client_active(&mut self, _actor: &ClientKey) {}
    /// Whether this adapter can keep new connections across a restart on request.
    fn persistence_supported(&self) -> bool {
        false
    }
    /// Reads the setting, or first changes it. It only affects connections made afterwards;
    /// existing and foreign profiles are never touched.
    fn persistence(
        &mut self,
        _actor: &ClientKey,
        _change: Option<bool>,
    ) -> Result<bool, WifiError> {
        Err(WifiError::Unsupported)
    }
    /// Whether the device offers the restart options `LastAccepted` and `DefaultProfile`.
    fn restart_supported(&self) -> bool {
        false
    }
    fn restart_state(&mut self, _actor: &ClientKey) -> Result<RestartState, WifiError> {
        Err(WifiError::Unsupported)
    }
    /// Starts changing the option; progress is polled like other tasks. A default needs an
    /// active Wi-Fi connection or access point.
    fn change_restart(
        &mut self,
        _actor: &ClientKey,
        _id: u64,
        _change: RestartChange,
    ) -> Result<(), WifiError> {
        Err(WifiError::Unsupported)
    }
}
