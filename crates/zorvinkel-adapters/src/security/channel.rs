use super::{SecureError, handshake::Role};
use std::time::Duration;
use zeroize::Zeroizing;
use zorvinkel_protocol::status::{MAX_PLAINTEXT, MAX_RECORD, record_payload, wrap_record};

pub const SESSION_LIFETIME: Duration = Duration::from_secs(600);
pub const MAX_RECORDS: usize = 1024;
/// Owns Noise counters and keys. It is neither Clone, Debug nor serializable.
pub struct SecureChannel {
    state: Option<snow::TransportState>,
    peer: [u8; 32],
    pub(super) role: Role,
    started: Duration,
    last_time: Duration,
    sent: usize,
    received: usize,
}
impl SecureChannel {
    pub(super) fn new(
        state: snow::TransportState,
        peer: [u8; 32],
        role: Role,
        now: Duration,
    ) -> Self {
        Self {
            state: Some(state),
            peer,
            role,
            started: now,
            last_time: now,
            sent: 0,
            received: 0,
        }
    }
    pub(super) fn started(&self) -> Duration {
        self.started
    }
    pub(super) fn is_closed(&self) -> bool {
        self.state.is_none()
    }
    pub(super) fn poll_timeout(&mut self, now: Duration) -> Result<(), SecureError> {
        let result = self.check_time(now);
        if result.is_err() {
            self.close();
        }
        result
    }
    pub fn peer_key(&self) -> &[u8; 32] {
        &self.peer
    }
    pub fn close(&mut self) {
        self.state = None;
    }
    pub fn encrypt(&mut self, plaintext: &[u8], now: Duration) -> Result<Vec<u8>, SecureError> {
        let result = (|| {
            self.check_time(now)?;
            if self.sent >= MAX_RECORDS || plaintext.is_empty() || plaintext.len() > MAX_PLAINTEXT {
                return Err(SecureError::Limit);
            }
            let mut bytes = vec![0; MAX_RECORD - 4];
            let length = self
                .state
                .as_mut()
                .ok_or(SecureError::Closed)?
                .write_message(plaintext, &mut bytes)?;
            self.sent += 1;
            Ok(wrap_record(&bytes[..length])?)
        })();
        if result.is_err() {
            self.close();
        }
        result
    }
    pub fn decrypt(
        &mut self,
        record: &[u8],
        now: Duration,
    ) -> Result<Zeroizing<Vec<u8>>, SecureError> {
        let result = (|| {
            self.check_time(now)?;
            if self.received >= MAX_RECORDS {
                return Err(SecureError::Limit);
            }
            let ciphertext = record_payload(record)?;
            let mut plaintext = Zeroizing::new(vec![0; MAX_RECORD - 4]);
            let length = self
                .state
                .as_mut()
                .ok_or(SecureError::Closed)?
                .read_message(ciphertext, &mut plaintext)?;
            if length == 0 || length > MAX_PLAINTEXT {
                return Err(SecureError::InvalidMessage);
            }
            plaintext.truncate(length);
            self.received += 1;
            Ok(plaintext)
        })();
        if result.is_err() {
            self.close();
        }
        result
    }
    pub fn lifetime_left(&self, now: Duration) -> Duration {
        SESSION_LIFETIME.saturating_sub(now.saturating_sub(self.started))
    }
    /// Records still allowed in the busier direction.
    pub fn records_left(&self) -> usize {
        MAX_RECORDS.saturating_sub(self.sent.max(self.received))
    }
    fn check_time(&mut self, now: Duration) -> Result<(), SecureError> {
        if self.state.is_none() {
            return Err(SecureError::Closed);
        }
        if now < self.last_time {
            return Err(SecureError::ClockMovedBackwards);
        }
        self.last_time = now;
        if now - self.started >= SESSION_LIFETIME {
            return Err(SecureError::TimedOut);
        }
        Ok(())
    }
}
