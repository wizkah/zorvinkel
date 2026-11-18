//! Bounded BLE framing and separate public probe and authenticated status endpoints.
use std::time::Duration;
use zorvinkel_protocol::{
    ble::{BASELINE_FRAME_LENGTH, FrameError, MessageKind, Reassembler, fragment_with_kind},
    probe,
};

#[cfg(feature = "ble-client")]
pub mod client;
#[cfg(all(feature = "ble-server", target_os = "linux"))]
pub mod server;

pub struct ProbeSession {
    receiver: Reassembler,
    kind: MessageKind,
    next_id: Option<u32>,
}
impl Default for ProbeSession {
    fn default() -> Self {
        Self::new()
    }
}
impl ProbeSession {
    pub fn new() -> Self {
        Self::with_kind(MessageKind::Probe)
    }
    pub fn with_kind(kind: MessageKind) -> Self {
        Self {
            receiver: Reassembler::with_kind(BASELINE_FRAME_LENGTH, kind)
                .expect("constant frame length is valid"),
            kind,
            next_id: Some(1),
        }
    }
    pub fn receive(&mut self, frame: &[u8], now: Duration) -> Result<Option<Vec<u8>>, FrameError> {
        let result = self.receiver.receive(frame, now);
        if result.is_err() {
            self.disconnect();
        }
        result
    }
    pub fn send(&mut self, payload: &[u8]) -> Result<Vec<Vec<u8>>, FrameError> {
        let id = self.next_id.ok_or(FrameError::Closed)?;
        let frames = fragment_with_kind(id, payload, BASELINE_FRAME_LENGTH, self.kind)?;
        self.next_id = id.checked_add(1);
        Ok(frames)
    }
    pub fn respond(&mut self, request: &[u8]) -> Result<Vec<Vec<u8>>, FrameError> {
        let Some(response) = probe::respond(request) else {
            self.disconnect();
            return Err(FrameError::UnsupportedKind);
        };
        self.send(response)
    }
    pub fn poll_timeout(&mut self, now: Duration) -> Result<(), FrameError> {
        let result = self.receiver.poll_timeout(now);
        if result.is_err() {
            self.disconnect();
        }
        result
    }
    pub fn disconnect(&mut self) {
        self.receiver.close();
        self.next_id = None;
    }
}

#[cfg(any(
    feature = "ble-client",
    all(feature = "ble-server", target_os = "linux")
))]
pub type BleResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Copy)]
pub struct Profile {
    pub service: u128,
    pub kind: MessageKind,
    pub write: u128,
    pub notify: u128,
}
pub const PROBE: Profile = Profile {
    kind: MessageKind::Probe,
    service: probe::SERVICE_UUID,
    write: probe::WRITE_UUID,
    notify: probe::NOTIFY_UUID,
};
pub const STATUS: Profile = Profile {
    kind: MessageKind::SecureRecord,
    service: 0x719e40a0_4a4b_4d75_9c08_a9cbb88f4200,
    write: 0x719e40a0_4a4b_4d75_9c08_a9cbb88f4201,
    notify: 0x719e40a0_4a4b_4d75_9c08_a9cbb88f4202,
};

#[cfg(all(feature = "ble-client", feature = "security"))]
pub mod secure_client;
#[cfg(all(any(unix, windows), feature = "ble-client", feature = "security"))]
pub mod setup;
