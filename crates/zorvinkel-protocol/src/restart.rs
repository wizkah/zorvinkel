//! Restart option of the device: which network configuration it applies after a restart.
//! Changing it is a task that is followed like other network tasks.
use crate::{status::CodecError, wifi};

/// The device lets authenticated clients read and choose the restart option.
pub const CAP_RESTART_POLICY: u32 = 2048;

/// What the device applies after a restart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartState {
    /// The last accepted configuration that was kept.
    LastAccepted,
    /// A fixed default: a saved Wi-Fi connection.
    DefaultWifi,
    /// A fixed default: the device's own Wi-Fi.
    DefaultAccessPoint,
}
impl RestartState {
    pub fn code(self) -> u8 {
        match self {
            Self::LastAccepted => 0,
            Self::DefaultWifi => 1,
            Self::DefaultAccessPoint => 2,
        }
    }
    pub fn from_code(code: u8) -> Result<Self, CodecError> {
        match code {
            0 => Ok(Self::LastAccepted),
            1 => Ok(Self::DefaultWifi),
            2 => Ok(Self::DefaultAccessPoint),
            _ => Err(CodecError::Malformed),
        }
    }
}
pub fn state_payload(state: RestartState) -> Vec<u8> {
    vec![state.code()]
}
pub fn read_state(payload: &[u8]) -> Result<RestartState, CodecError> {
    match payload {
        [code] => RestartState::from_code(*code),
        _ => Err(CodecError::Malformed),
    }
}

/// The change a client asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartChange {
    /// Apply the last accepted configuration (that was kept) after a restart.
    LastAccepted,
    /// Make the configuration that is active now the default for every restart.
    CurrentAsDefault,
}
pub struct RestartRequest {
    pub id: u64,
    pub change: RestartChange,
}
pub fn set_payload(id: u64, change: RestartChange) -> Result<Vec<u8>, CodecError> {
    let mut bytes = wifi::id_payload(id)?;
    bytes.push(match change {
        RestartChange::LastAccepted => 0,
        RestartChange::CurrentAsDefault => 1,
    });
    Ok(bytes)
}
pub fn read_set(payload: &[u8]) -> Result<RestartRequest, CodecError> {
    if payload.len() != 9 {
        return Err(CodecError::Malformed);
    }
    let change = match payload[8] {
        0 => RestartChange::LastAccepted,
        1 => RestartChange::CurrentAsDefault,
        _ => return Err(CodecError::Malformed),
    };
    Ok(RestartRequest {
        id: wifi::read_id(&payload[..8])?,
        change,
    })
}
