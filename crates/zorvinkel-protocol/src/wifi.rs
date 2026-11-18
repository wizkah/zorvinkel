//! Optional volatile Wi-Fi extension; credentials remain borrowed from decrypted storage.
use crate::status::CodecError;
pub const CAP_WIFI_TEMPORARY: u32 = 4;
/// The device lets authenticated clients choose whether new connections survive a restart.
pub const CAP_WIFI_PERSISTENCE: u32 = 8;
pub const MAX_NETWORKS: usize = 64;
pub struct WifiConnect<'a> {
    pub id: u64,
    pub ssid: &'a str,
    pub password: &'a str,
}

pub fn connect_payload(id: u64, ssid: &str, password: &str) -> Result<Vec<u8>, CodecError> {
    validate_credentials(ssid, password)?;
    let mut bytes = id_payload(id)?;
    bytes.push(ssid.len() as u8);
    bytes.extend(ssid.as_bytes());
    bytes.push(password.len() as u8);
    bytes.extend(password.as_bytes());
    Ok(bytes)
}
/// Persistence request: 0 = do not keep new connections, 1 = keep them, 2 = query only.
pub fn persistence_payload(change: Option<bool>) -> Vec<u8> {
    vec![change.map_or(2, u8::from)]
}
pub fn read_persistence(payload: &[u8]) -> Result<Option<bool>, CodecError> {
    match payload {
        [0] => Ok(Some(false)),
        [1] => Ok(Some(true)),
        [2] => Ok(None),
        _ => Err(CodecError::Malformed),
    }
}
pub fn id_payload(id: u64) -> Result<Vec<u8>, CodecError> {
    if id == 0 {
        return Err(CodecError::Malformed);
    }
    Ok(id.to_le_bytes().to_vec())
}
pub fn read_id(payload: &[u8]) -> Result<u64, CodecError> {
    let id = u64::from_le_bytes(payload.try_into().map_err(|_| CodecError::Malformed)?);
    if id == 0 {
        return Err(CodecError::Malformed);
    }
    Ok(id)
}
pub fn read_connect(payload: &[u8]) -> Result<WifiConnect<'_>, CodecError> {
    if payload.len() < 10 {
        return Err(CodecError::Malformed);
    }
    let id = read_id(&payload[..8])?;
    let ssid_end = 9 + usize::from(payload[8]);
    if ssid_end >= payload.len() {
        return Err(CodecError::Malformed);
    }
    let password_end = ssid_end + 1 + usize::from(payload[ssid_end]);
    if password_end != payload.len() {
        return Err(CodecError::Malformed);
    }
    let ssid = std::str::from_utf8(&payload[9..ssid_end]).map_err(|_| CodecError::Malformed)?;
    let password =
        std::str::from_utf8(&payload[ssid_end + 1..]).map_err(|_| CodecError::Malformed)?;
    validate_credentials(ssid, password)?;
    Ok(WifiConnect { id, ssid, password })
}
fn validate_credentials(ssid: &str, password: &str) -> Result<(), CodecError> {
    if ssid.is_empty()
        || ssid.len() > 32
        || ssid.chars().any(char::is_control)
        || !(8..=63).contains(&password.len())
        || password.chars().any(char::is_control)
    {
        return Err(CodecError::Malformed);
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WifiTask {
    pub id: u64,
    pub state: u8,
    pub networks: Vec<String>,
}
pub fn encode_task(task: &WifiTask) -> Result<Vec<u8>, CodecError> {
    if task.state > 5
        || task.networks.len() > MAX_NETWORKS
        || (task.state != 1 && !task.networks.is_empty())
    {
        return Err(CodecError::Malformed);
    }
    let mut bytes = id_payload(task.id)?;
    bytes.extend([task.state, task.networks.len() as u8]);
    for ssid in &task.networks {
        if ssid.is_empty() || ssid.len() > 32 || ssid.chars().any(char::is_control) {
            return Err(CodecError::Malformed);
        }
        bytes.push(ssid.len() as u8);
        bytes.extend(ssid.as_bytes());
    }
    Ok(bytes)
}
pub fn decode_task(payload: &[u8]) -> Result<WifiTask, CodecError> {
    if payload.len() < 10 {
        return Err(CodecError::Malformed);
    }
    let mut task = WifiTask {
        id: read_id(&payload[..8])?,
        state: payload[8],
        networks: Vec::new(),
    };
    let mut offset = 10;
    for _ in 0..payload[9] {
        let length = usize::from(*payload.get(offset).ok_or(CodecError::Malformed)?);
        offset += 1;
        let ssid = payload
            .get(offset..offset + length)
            .ok_or(CodecError::Malformed)?;
        task.networks.push(
            std::str::from_utf8(ssid)
                .map_err(|_| CodecError::Malformed)?
                .into(),
        );
        offset += length;
    }
    if offset != payload.len() {
        return Err(CodecError::Malformed);
    }
    encode_task(&task)?;
    Ok(task)
}
