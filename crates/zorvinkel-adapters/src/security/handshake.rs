use super::{ChannelContext, PROFILE, SecureError, channel::SecureChannel};
use crate::identity::IdentityHandle;
use std::time::Duration;
use zeroize::Zeroizing;
use zorvinkel_protocol::status::{record_payload, wrap_record};

pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Client,
    Device,
}

pub struct Handshake {
    state: Option<snow::HandshakeState>,
    role: Role,
    step: usize,
    started: Duration,
    last_time: Duration,
}
impl Handshake {
    pub fn new(
        identity: &IdentityHandle,
        role: Role,
        context: ChannelContext,
        now: Duration,
    ) -> Result<Self, SecureError> {
        super::identity::validate(identity)?;
        let prologue = context.prologue();
        let builder = snow::Builder::new(PROFILE.parse().map_err(|_| SecureError::Crypto)?)
            .local_private_key(&identity.material.private)?
            .prologue(&prologue)?;
        let state = match role {
            Role::Client => builder.build_initiator()?,
            Role::Device => builder.build_responder()?,
        };
        Ok(Self {
            state: Some(state),
            role,
            step: 0,
            started: now,
            last_time: now,
        })
    }
    pub fn write(&mut self, now: Duration) -> Result<Vec<u8>, SecureError> {
        let result = (|| {
            self.check_time(now)?;
            if !self.is_write_turn() || self.step >= 3 {
                return Err(SecureError::InvalidMessage);
            }
            let mut bytes = [0; 96];
            let length = self
                .state
                .as_mut()
                .ok_or(SecureError::Closed)?
                .write_message(&[], &mut bytes)?;
            if length != [32, 96, 64][self.step] {
                return Err(SecureError::InvalidMessage);
            }
            self.step += 1;
            Ok(wrap_record(&bytes[..length])?)
        })();
        if result.is_err() {
            self.state = None;
        }
        result
    }
    pub fn read(&mut self, record: &[u8], now: Duration) -> Result<(), SecureError> {
        let result = (|| {
            self.check_time(now)?;
            if self.is_write_turn() || self.step >= 3 {
                return Err(SecureError::InvalidMessage);
            }
            let bytes = record_payload(record)?;
            if bytes.len() != [32, 96, 64][self.step] {
                return Err(SecureError::InvalidMessage);
            }
            let mut payload = Zeroizing::new([0; 96]);
            if self
                .state
                .as_mut()
                .ok_or(SecureError::Closed)?
                .read_message(bytes, &mut payload[..])?
                != 0
            {
                return Err(SecureError::InvalidMessage);
            }
            self.step += 1;
            Ok(())
        })();
        if result.is_err() {
            self.state = None;
        }
        result
    }
    pub fn peer_key(&self) -> Result<[u8; 32], SecureError> {
        self.state
            .as_ref()
            .ok_or(SecureError::Closed)?
            .get_remote_static()
            .ok_or(SecureError::InvalidMessage)?
            .try_into()
            .map_err(|_| SecureError::InvalidIdentity)
    }
    pub fn finish(mut self, now: Duration) -> Result<SecureChannel, SecureError> {
        self.check_time(now)?;
        let peer = self.peer_key()?;
        let state = self.state.take().ok_or(SecureError::Closed)?;
        if self.step != 3 || !state.is_handshake_finished() {
            return Err(SecureError::InvalidMessage);
        }
        Ok(SecureChannel::new(
            state.into_transport_mode()?,
            peer,
            self.role,
            now,
        ))
    }
    fn is_write_turn(&self) -> bool {
        (self.role == Role::Client) == self.step.is_multiple_of(2)
    }
    fn check_time(&mut self, now: Duration) -> Result<(), SecureError> {
        if self.state.is_none() {
            return Err(SecureError::Closed);
        }
        if now < self.last_time {
            return Err(SecureError::ClockMovedBackwards);
        }
        self.last_time = now;
        if now - self.started >= HANDSHAKE_TIMEOUT {
            return Err(SecureError::TimedOut);
        }
        Ok(())
    }
}
