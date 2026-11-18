//! Access point extension. Credentials stay borrowed from decrypted payloads where possible
//! and never appear in `Debug` output.
use crate::{status::CodecError, wifi};

/// The device can provide its own Wi-Fi network.
pub const CAP_ACCESS_POINT: u32 = 32;

#[derive(Clone, PartialEq, Eq)]
pub struct ApCredentials {
    pub ssid: String,
    pub password: String,
}
impl std::fmt::Debug for ApCredentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ApCredentials")
            .field("ssid", &self.ssid)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// WPA2 passphrases are 8–63 printable ASCII characters.
pub fn validate(ssid: &str, password: &str) -> Result<(), CodecError> {
    if ssid.is_empty()
        || ssid.len() > 32
        || ssid.chars().any(char::is_control)
        || !(8..=63).contains(&password.len())
        || !password.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
    {
        return Err(CodecError::Malformed);
    }
    Ok(())
}
pub fn id_payload(id: u64) -> Result<Vec<u8>, CodecError> {
    wifi::id_payload(id)
}
pub fn read_id(payload: &[u8]) -> Result<u64, CodecError> {
    wifi::read_id(payload)
}
pub fn credentials_payload(ssid: &str, password: &str) -> Result<Vec<u8>, CodecError> {
    validate(ssid, password)?;
    let mut bytes = vec![ssid.len() as u8];
    bytes.extend(ssid.as_bytes());
    bytes.push(password.len() as u8);
    bytes.extend(password.as_bytes());
    Ok(bytes)
}
pub fn read_credentials(payload: &[u8]) -> Result<ApCredentials, CodecError> {
    let ssid_length = usize::from(*payload.first().ok_or(CodecError::Malformed)?);
    let ssid = payload
        .get(1..1 + ssid_length)
        .ok_or(CodecError::Malformed)?;
    let password_length = usize::from(*payload.get(1 + ssid_length).ok_or(CodecError::Malformed)?);
    let password = payload
        .get(2 + ssid_length..)
        .filter(|rest| rest.len() == password_length)
        .ok_or(CodecError::Malformed)?;
    let credentials = ApCredentials {
        ssid: std::str::from_utf8(ssid)
            .map_err(|_| CodecError::Malformed)?
            .into(),
        password: std::str::from_utf8(password)
            .map_err(|_| CodecError::Malformed)?
            .into(),
    };
    validate(&credentials.ssid, &credentials.password)?;
    Ok(credentials)
}
/// An empty payload means the device has no stored credentials yet.
pub fn encode_stored(credentials: &Option<ApCredentials>) -> Result<Vec<u8>, CodecError> {
    credentials.as_ref().map_or(Ok(Vec::new()), |stored| {
        credentials_payload(&stored.ssid, &stored.password)
    })
}
pub fn decode_stored(payload: &[u8]) -> Result<Option<ApCredentials>, CodecError> {
    if payload.is_empty() {
        return Ok(None);
    }
    read_credentials(payload).map(Some)
}
