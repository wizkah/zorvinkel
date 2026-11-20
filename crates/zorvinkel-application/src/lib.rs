//! Client and device use cases and their infrastructure ports.

pub mod client;
pub mod device;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortError {
    Unavailable,
    Disconnected,
    Unauthorized,
}

impl std::fmt::Display for PortError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "service unavailable",
            Self::Disconnected => "device disconnected",
            Self::Unauthorized => "client unauthorized",
        })
    }
}

impl std::error::Error for PortError {}

pub mod network;
pub mod registration;
pub mod state;

pub mod trust;

pub mod setup;

pub mod status;

pub mod wifi;

pub mod device_names;

pub mod power;

pub mod system;

pub mod services;

pub mod tunnel;
