//! Authorized shutdown and restart of the device. Authorization is the caller's duty.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerAction {
    Shutdown,
    Restart,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerError {
    /// A power action is already scheduled.
    Busy,
    Unavailable,
}
pub trait PowerControl {
    /// Schedules the action after the caller has been able to deliver its acknowledgement.
    fn schedule(&mut self, action: PowerAction) -> Result<(), PowerError>;
}
