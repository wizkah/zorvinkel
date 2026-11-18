//! What the setup worker needs from a network stack. Both NetworkManager and
//! wpa_supplicant/dhcpcd implement it, so the worker does not know which one it drives.
use zorvinkel_application::network::NetworkControl;
use zorvinkel_core::network::{NetworkConfiguration, NetworkError, NetworkIntent, ProfileId};

pub trait StackControl: NetworkControl {
    /// Whether the stack can run the device's own Wi-Fi.
    fn access_point_supported(&self) -> bool;
    /// Adds a profile for a station connection that is neither active nor saved yet.
    fn prepare_wifi(&mut self, ssid: &str, password: &str) -> Result<ProfileId, NetworkError>;
    /// Adds a profile that makes the interface an access point.
    fn prepare_access_point(
        &mut self,
        ssid: &str,
        password: &str,
    ) -> Result<ProfileId, NetworkError>;
    fn scan(&mut self) -> Result<Vec<String>, NetworkError>;
    /// Removes profiles created by this service that are not active.
    fn discard_inactive_profiles(&mut self) -> Result<(), NetworkError>;
    /// Keeps the active profile created by this service across restarts of the device.
    fn persist_active(&mut self) -> Result<(), NetworkError>;
    /// True while the connection is one of the service's own volatile profiles.
    fn owned_connection_active(&mut self) -> Result<bool, NetworkError>;
    /// Lets the stack pick a saved network again, e.g. after the access point was switched off.
    fn reconnect(&mut self) -> Result<(), NetworkError>;
    /// Drops every volatile profile and returns to the saved configuration.
    fn revert_volatile(&mut self) -> Result<(), NetworkError>;
    /// The saved (not volatile) station profile of this network, if the stack knows one.
    fn saved_wifi_profile(&mut self, ssid: &str) -> Result<Option<ProfileId>, NetworkError>;
    /// Makes sure the active station connection survives a restart of the device. A profile
    /// that this service created is saved; a profile that already existed is left alone.
    fn keep_active(&mut self) -> Result<(), NetworkError>;
    /// Lets the stack use its other saved networks again after `apply` selected one.
    fn release_selection(&mut self) -> Result<(), NetworkError> {
        Ok(())
    }
    /// Connects to the saved network with this SSID. Not journaled: no client asked for it,
    /// and it only repeats a configuration a client fixed as the default.
    fn apply_default_wifi(&mut self, ssid: &str) -> Result<(), NetworkError> {
        let profile = self
            .saved_wifi_profile(ssid)?
            .ok_or(NetworkError::Unavailable)?;
        if self.observe()? == NetworkConfiguration::Wifi(profile.clone()) {
            return Ok(());
        }
        self.apply(&NetworkIntent::ConnectWifi(profile))?;
        self.release_selection()
    }
}

/// The SSID that both stacks append as hex to their profile references.
pub fn profile_ssid(profile: &ProfileId) -> Option<String> {
    let hex = profile.as_str().rsplit(':').next()?;
    if hex.is_empty() || hex.len() > 64 || !hex.len().is_multiple_of(2) || !hex.is_ascii() {
        return None;
    }
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect::<Option<Vec<_>>>()?;
    let ssid = String::from_utf8(bytes).ok()?;
    (!ssid.chars().any(char::is_control)).then_some(ssid)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_ssid_is_read_from_either_profile_reference() {
        let wpa = ProfileId::new("wpa:3:4576656e74".into()).unwrap();
        let nm = ProfileId::new("nm:1234-abcd:4576656e74".into()).unwrap();
        assert_eq!(profile_ssid(&wpa).as_deref(), Some("Event"));
        assert_eq!(profile_ssid(&nm).as_deref(), Some("Event"));
        for broken in ["wpa:3:", "wpa:3:4", "wpa:3:zz", "wpa:3:0a"] {
            assert_eq!(profile_ssid(&ProfileId::new(broken.into()).unwrap()), None);
        }
    }
}
