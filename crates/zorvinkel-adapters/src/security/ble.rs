//! Per-connection responder shared by the native endpoint and transport tests.
use super::{
    ChannelContext, SecureError,
    handshake::{HANDSHAKE_TIMEOUT, Handshake, Role},
    service::{DeviceSession, ServiceTime, SessionRegistry},
};
use crate::identity::IdentityHandle;
use std::time::Duration;
use zorvinkel_application::{device::NetworkStatusSource, state::StateStore};

enum Phase {
    First(Handshake),
    Final(Handshake),
    Established(DeviceSession),
    Closed,
}
pub struct Responder {
    phase: Phase,
    started: Duration,
    over_ip: bool,
}
impl Responder {
    pub fn new(identity: &IdentityHandle, now: Duration) -> Result<Self, SecureError> {
        Self::with_context(identity, ChannelContext::Ble, now)
    }
    /// Responder of the IP endpoint; its session offers the same operations as over Bluetooth.
    pub fn new_ip(identity: &IdentityHandle, now: Duration) -> Result<Self, SecureError> {
        Self::with_context(identity, ChannelContext::Ip, now)
    }
    fn with_context(
        identity: &IdentityHandle,
        context: ChannelContext,
        now: Duration,
    ) -> Result<Self, SecureError> {
        Ok(Self {
            phase: Phase::First(Handshake::new(identity, Role::Device, context, now)?),
            started: now,
            over_ip: context == ChannelContext::Ip,
        })
    }
    pub fn receive(
        &mut self,
        record: &[u8],
        store: &mut impl StateStore,
        network: &mut impl NetworkStatusSource,
        time: ServiceTime,
        sessions: &SessionRegistry,
    ) -> Result<Option<Vec<u8>>, SecureError> {
        let phase = std::mem::replace(&mut self.phase, Phase::Closed);
        match phase {
            Phase::First(mut handshake) => {
                handshake.read(record, time.elapsed)?;
                let response = handshake.write(time.elapsed)?;
                self.phase = Phase::Final(handshake);
                Ok(Some(response))
            }
            Phase::Final(mut handshake) => {
                handshake.read(record, time.elapsed)?;
                let channel = handshake.finish(time.elapsed)?;
                self.phase = Phase::Established(if self.over_ip {
                    DeviceSession::new_ip(channel, sessions)?
                } else {
                    DeviceSession::new(channel, sessions)?
                });
                Ok(None)
            }
            Phase::Established(mut session) => {
                let response = session.handle(record, store, network, time)?;
                self.phase = Phase::Established(session);
                Ok(Some(response))
            }
            Phase::Closed => Err(SecureError::Closed),
        }
    }
    pub fn closed(&self, now: Duration) -> bool {
        match &self.phase {
            Phase::First(_) | Phase::Final(_) => {
                now < self.started || now.saturating_sub(self.started) >= HANDSHAKE_TIMEOUT
            }
            Phase::Established(session) => session.is_closed(),
            Phase::Closed => true,
        }
    }
}
