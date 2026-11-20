//! Terminal tunnel to the device's local `sshd` (ADR-031). Authorization is the caller's duty.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TunnelRefusal {
    /// The operator switched the terminal off, or `sshd` cannot be reached.
    Unavailable,
    /// Too many openings recently.
    Busy,
}

/// Decides whether the device offers a tunnel at this moment.
pub trait TunnelControl {
    /// The device is willing to open a tunnel: switch on and `sshd` reachable on loopback.
    fn admit(&mut self) -> Result<(), TunnelRefusal>;
}
