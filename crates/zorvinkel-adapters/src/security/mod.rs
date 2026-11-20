//! Noise-backed authenticated status channel.
pub mod channel;
pub mod handshake;
pub mod identity;
pub mod service;
pub mod simulation;
pub mod tunnel;

pub const PROFILE: &str = "Noise_XX_25519_ChaChaPoly_SHA256";
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecureError {
    Crypto,
    InvalidIdentity,
    InvalidMessage,
    ProtocolVersion,
    Closed,
    TimedOut,
    ClockMovedBackwards,
    Limit,
    Trust,
    Storage,
    Unauthorized,
    Busy,
}
impl std::fmt::Display for SecureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "secure channel error: {self:?}")
    }
}
impl std::error::Error for SecureError {}
impl From<snow::Error> for SecureError {
    fn from(_: snow::Error) -> Self {
        Self::Crypto
    }
}
impl From<zorvinkel_protocol::status::CodecError> for SecureError {
    fn from(error: zorvinkel_protocol::status::CodecError) -> Self {
        match error {
            zorvinkel_protocol::status::CodecError::Version => Self::ProtocolVersion,
            _ => Self::InvalidMessage,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelContext {
    Simulation,
    Ble,
    Ip,
}
impl ChannelContext {
    fn prologue(self) -> Vec<u8> {
        let suffix = match self {
            Self::Simulation => "simulation",
            Self::Ble => "ble",
            Self::Ip => "ip",
        };
        format!("zorvinkel/status/1.0/{suffix}").into_bytes()
    }
}

pub mod ble;
#[cfg(any(unix, windows))]
pub mod storage;
