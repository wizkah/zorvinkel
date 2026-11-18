//! Reads the key of a Wi-Fi profile that Windows has saved (WLAN API, `WlanGetProfile`).
//! Windows hands out the plaintext key only to an elevated process for profiles saved for all
//! users; otherwise the call fails and the password is unavailable. The only unsafe code of
//! this feature lives here.
#![allow(unsafe_code)]
use std::ptr::{null, null_mut};
use windows_sys::{
    Win32::{
        Foundation::{ERROR_SUCCESS, HANDLE},
        NetworkManagement::WiFi::{
            WLAN_INTERFACE_INFO_LIST, WLAN_PROFILE_GET_PLAINTEXT_KEY, WlanCloseHandle,
            WlanEnumInterfaces, WlanFreeMemory, WlanGetProfile, WlanOpenHandle,
        },
    },
    core::GUID,
};
use zeroize::Zeroizing;

struct Session(HANDLE);
impl Drop for Session {
    fn drop(&mut self) {
        // SAFETY: the handle came from `WlanOpenHandle` and is closed once.
        unsafe { WlanCloseHandle(self.0, null()) };
    }
}

/// The key material of the saved profile with exactly this name, if Windows releases it.
pub(crate) fn profile_key(name: &str) -> Option<Zeroizing<String>> {
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut version = 0;
    let mut raw: HANDLE = null_mut();
    // SAFETY: both out pointers are valid for the call.
    if unsafe { WlanOpenHandle(2, null(), &mut version, &mut raw) } != ERROR_SUCCESS {
        return None;
    }
    let session = Session(raw);
    let mut list: *mut WLAN_INTERFACE_INFO_LIST = null_mut();
    // SAFETY: `list` is a valid out pointer and the handle is open.
    if unsafe { WlanEnumInterfaces(session.0, null(), &mut list) } != ERROR_SUCCESS {
        return None;
    }
    // SAFETY: on success the list holds `dwNumberOfItems` entries and lives until freed below.
    let guids: Vec<GUID> = unsafe {
        let count = (*list).dwNumberOfItems as usize;
        std::slice::from_raw_parts((*list).InterfaceInfo.as_ptr(), count)
            .iter()
            .map(|info| info.InterfaceGuid)
            .collect()
    };
    // SAFETY: `list` came from `WlanEnumInterfaces` and is not used afterwards.
    unsafe { WlanFreeMemory(list.cast()) };
    guids.iter().find_map(|guid| {
        let mut xml: *mut u16 = null_mut();
        let mut flags = WLAN_PROFILE_GET_PLAINTEXT_KEY;
        let mut access = 0;
        // SAFETY: all pointers are valid; the name is NUL-terminated.
        let status = unsafe {
            WlanGetProfile(
                session.0,
                guid,
                name.as_ptr(),
                null(),
                &mut xml,
                &mut flags,
                &mut access,
            )
        };
        if status != ERROR_SUCCESS || xml.is_null() {
            return None;
        }
        // SAFETY: on success `xml` is a NUL-terminated UTF-16 string owned by the system.
        let (text, length) = unsafe {
            let mut length = 0;
            while *xml.add(length) != 0 {
                length += 1;
            }
            (
                Zeroizing::new(String::from_utf16_lossy(std::slice::from_raw_parts(
                    xml, length,
                ))),
                length,
            )
        };
        // SAFETY: wipe the system's copy of the secret, then free what the call returned.
        unsafe {
            std::ptr::write_bytes(xml, 0, length);
            WlanFreeMemory(xml.cast());
        }
        key_material(&text)
    })
}

/// The plaintext key of a profile document; protected (encrypted) keys are not usable.
fn key_material(xml: &str) -> Option<Zeroizing<String>> {
    if tag(xml, "protected")? != "false" {
        return None;
    }
    let key = unescape(tag(xml, "keyMaterial")?);
    Some(Zeroizing::new(key))
}
fn tag<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let rest = &xml[xml.find(&format!("<{name}>"))? + name.len() + 2..];
    Some(&rest[..rest.find(&format!("</{name}>"))?])
}
fn unescape(text: &str) -> String {
    let mut out = text.to_owned();
    for (entity, replacement) in [
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&apos;", "'"),
    ] {
        out = out.replace(entity, replacement);
    }
    out.replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(protected: &str, key: &str) -> String {
        format!(
            "<WLANProfile><MSM><security><sharedKey><keyType>passPhrase</keyType>\
             <protected>{protected}</protected><keyMaterial>{key}</keyMaterial>\
             </sharedKey></security></MSM></WLANProfile>"
        )
    }
    #[test]
    fn plaintext_key_is_read_and_unescaped() {
        let xml = profile("false", "a&amp;b&lt;c&gt;&quot;d&apos;");
        assert_eq!(key_material(&xml).unwrap().as_str(), "a&b<c>\"d'");
    }
    #[test]
    fn protected_or_missing_keys_are_unavailable() {
        assert!(key_material(&profile("true", "00AB")).is_none());
        assert!(key_material("<WLANProfile/>").is_none());
        assert!(key_material("<protected>false</protected>").is_none());
    }
}
