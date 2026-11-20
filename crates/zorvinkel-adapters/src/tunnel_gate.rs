//! The device's decision to open a terminal tunnel (ADR-030, ADR-031): the operator switch,
//! whether `sshd` answers on loopback, and a limit on how often tunnels are opened.
use std::{
    collections::VecDeque,
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    time::{Duration, Instant},
};
use zorvinkel_application::tunnel::{TunnelControl, TunnelRefusal};

/// The terminal over Bluetooth is on unless this file exists (ADR-030: on by default, the
/// operator can switch it off at the device).
pub const DISABLE_FILE: &str = "/etc/zorvinkel/terminal-disabled";
/// `sshd` as the device reaches it; a tunnel carries no target of its own.
pub const SSHD_ADDRESS: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 22);
/// Openings allowed per window. The numbers are provisional (ADR-031 leaves them open).
const MAX_OPENINGS: usize = 6;
const OPENING_WINDOW: Duration = Duration::from_secs(60);
const REACH_TIMEOUT: Duration = Duration::from_millis(500);

pub struct TunnelGate {
    disable_file: PathBuf,
    target: SocketAddr,
    openings: VecDeque<Instant>,
}
impl TunnelGate {
    pub fn new(disable_file: impl Into<PathBuf>, target: SocketAddr) -> Self {
        Self {
            disable_file: disable_file.into(),
            target,
            openings: VecDeque::new(),
        }
    }
    pub fn system() -> Self {
        Self::new(DISABLE_FILE, SSHD_ADDRESS)
    }
}
impl TunnelControl for TunnelGate {
    fn admit(&mut self) -> Result<(), TunnelRefusal> {
        if self.disable_file.exists() {
            return Err(TunnelRefusal::Unavailable);
        }
        let now = Instant::now();
        while self
            .openings
            .front()
            .is_some_and(|opened| now.duration_since(*opened) >= OPENING_WINDOW)
        {
            self.openings.pop_front();
        }
        if self.openings.len() >= MAX_OPENINGS {
            return Err(TunnelRefusal::Busy);
        }
        // Checked before counting, so a missing `sshd` does not use up the allowance; the
        // client's own retries are limited by the same window once it answers.
        TcpStream::connect_timeout(&self.target, REACH_TIMEOUT)
            .map_err(|_| TunnelRefusal::Unavailable)?;
        self.openings.push_back(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn listener() -> (TcpListener, SocketAddr) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        (listener, address)
    }
    fn missing_file() -> PathBuf {
        std::env::temp_dir().join(format!("zorvinkel-no-such-{}", std::process::id()))
    }

    #[test]
    fn opens_when_the_switch_is_on_and_sshd_answers() {
        let (_listener, address) = listener();
        assert_eq!(TunnelGate::new(missing_file(), address).admit(), Ok(()));
    }
    #[test]
    fn the_operator_switch_wins() {
        let (_listener, address) = listener();
        let file = std::env::temp_dir().join(format!("zorvinkel-off-{}", std::process::id()));
        std::fs::write(&file, b"").unwrap();
        let result = TunnelGate::new(&file, address).admit();
        std::fs::remove_file(&file).unwrap();
        assert_eq!(result, Err(TunnelRefusal::Unavailable));
    }
    #[test]
    fn a_missing_sshd_is_unavailable_and_costs_nothing() {
        let (listener, address) = listener();
        drop(listener);
        let mut gate = TunnelGate::new(missing_file(), address);
        for _ in 0..(MAX_OPENINGS + 2) {
            assert_eq!(gate.admit(), Err(TunnelRefusal::Unavailable));
        }
    }
    #[test]
    fn too_many_openings_in_a_window_are_busy() {
        let (_listener, address) = listener();
        let mut gate = TunnelGate::new(missing_file(), address);
        for _ in 0..MAX_OPENINGS {
            assert_eq!(gate.admit(), Ok(()));
        }
        assert_eq!(gate.admit(), Err(TunnelRefusal::Busy));
        // The window moves on.
        gate.openings
            .iter_mut()
            .for_each(|opened| *opened -= OPENING_WINDOW);
        assert_eq!(gate.admit(), Ok(()));
    }
}
