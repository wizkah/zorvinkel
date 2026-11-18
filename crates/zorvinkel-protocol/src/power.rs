//! Power commands for the connected device. They carry no secrets and need an authorized session.
use crate::status::CodecError;

/// The device offers to shut down or restart on request of an authorized client.
pub const CAP_POWER: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerCommand {
    Shutdown,
    Restart,
}

/// Request and acknowledgement payload: 1 = shut down, 2 = restart.
pub fn power_payload(command: PowerCommand) -> Vec<u8> {
    vec![match command {
        PowerCommand::Shutdown => 1,
        PowerCommand::Restart => 2,
    }]
}
pub fn read_power(payload: &[u8]) -> Result<PowerCommand, CodecError> {
    match payload {
        [1] => Ok(PowerCommand::Shutdown),
        [2] => Ok(PowerCommand::Restart),
        _ => Err(CodecError::Malformed),
    }
}
