//! Complete encrypted records over an in-memory transport. No sockets or BLE.
use super::{
    ChannelContext, SecureError,
    handshake::{Handshake, Role},
    service::{ClientSession, DeviceSession, SessionRegistry},
};
use crate::identity::IdentityHandle;
use std::time::Duration;
use zorvinkel_application::trust::{DeviceTrustStore, check_device, verify_device};
use zorvinkel_core::trust::{DeviceKey, TrustMode};

pub fn connect(
    client: &IdentityHandle,
    device: &IdentityHandle,
    trust: &mut impl DeviceTrustStore,
    mode: TrustMode,
    context: ChannelContext,
    now: Duration,
    sessions: &SessionRegistry,
) -> Result<(ClientSession, DeviceSession), SecureError> {
    let mut client = Handshake::new(client, Role::Client, context, now)?;
    let mut device = Handshake::new(device, Role::Device, context, now)?;
    device.read(&client.write(now)?, now)?;
    client.read(&device.write(now)?, now)?;
    let peer =
        DeviceKey::new(client.peer_key()?.to_vec()).map_err(|_| SecureError::InvalidIdentity)?;
    check_device(trust, peer.clone(), mode).map_err(trust_error)?;
    device.read(&client.write(now)?, now)?;
    let client = client.finish(now)?;
    let device = device.finish(now)?;
    verify_device(trust, peer, mode).map_err(trust_error)?;
    Ok((
        ClientSession::new(client)?,
        DeviceSession::new(device, sessions)?,
    ))
}

fn trust_error(error: zorvinkel_application::state::ApplicationError) -> SecureError {
    match error {
        zorvinkel_application::state::ApplicationError::Storage(_) => SecureError::Storage,
        _ => SecureError::Trust,
    }
}
