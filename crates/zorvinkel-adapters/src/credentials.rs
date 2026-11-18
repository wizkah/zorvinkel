//! Client-side access to Wi-Fi passwords stored by the operating system.
//! Only read on explicit request; the system asks the user for approval itself.
use zorvinkel_application::setup::{ClientCredentialSource, SetupError};

pub struct SystemCredentialSource;

#[cfg(target_os = "macos")]
impl ClientCredentialSource for SystemCredentialSource {
    fn saved_wifi_password(&mut self, ssid: &str) -> Result<String, SetupError> {
        // CoreWLAN offers no safe binding for this call.
        #![allow(unsafe_code)]
        use objc2::rc::autoreleasepool;
        use objc2_core_wlan::{CWKeychainDomain, CWKeychainFindWiFiPassword};
        use objc2_foundation::{NSData, NSString};
        use zorvinkel_application::setup::validate_wifi;

        if ssid.is_empty() || ssid.len() > 32 || ssid.chars().any(char::is_control) {
            return Err(SetupError::InvalidInput);
        }
        let name = NSData::with_bytes(ssid.as_bytes());
        // The user keychain needs no extra approval, so it goes first. macOS asks the user
        // itself before it releases a password from either keychain.
        for domain in [CWKeychainDomain::User, CWKeychainDomain::System] {
            let found = autoreleasepool(|_| {
                let mut password: *mut NSString = std::ptr::null_mut();
                // SAFETY: `password` is a valid out pointer; the returned string is
                // autoreleased and only borrowed inside this pool.
                let status = unsafe { CWKeychainFindWiFiPassword(domain, &name, &mut password) };
                // SAFETY: non-null on success and valid until the pool drains.
                let password = unsafe { password.as_ref() }?;
                (status == 0).then(|| password.to_string())
            });
            if let Some(password) = found {
                return if validate_wifi(ssid, &password).is_ok() {
                    Ok(password)
                } else {
                    Err(SetupError::CredentialsUnavailable)
                };
            }
        }
        Err(SetupError::CredentialsUnavailable)
    }
}

/// Windows releases the key only for profiles the process may read; otherwise it is unavailable.
#[cfg(windows)]
impl ClientCredentialSource for SystemCredentialSource {
    fn saved_wifi_password(&mut self, ssid: &str) -> Result<String, SetupError> {
        use zorvinkel_application::setup::validate_wifi;
        if ssid.is_empty() || ssid.len() > 32 || ssid.chars().any(char::is_control) {
            return Err(SetupError::InvalidInput);
        }
        let password =
            crate::windows_wifi::profile_key(ssid).ok_or(SetupError::CredentialsUnavailable)?;
        validate_wifi(ssid, &password).map_err(|_| SetupError::CredentialsUnavailable)?;
        Ok(password.to_string())
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
impl ClientCredentialSource for SystemCredentialSource {
    fn saved_wifi_password(&mut self, _ssid: &str) -> Result<String, SetupError> {
        Err(SetupError::CredentialsUnavailable)
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_network_names_without_asking_the_system() {
        for ssid in ["", "bad\nname", &"x".repeat(33)] {
            assert_eq!(
                SystemCredentialSource.saved_wifi_password(ssid),
                Err(SetupError::InvalidInput)
            );
        }
    }
}
