use std::time::Duration;
use zorvinkel_adapters::{
    identity::IdentityHandle,
    memory::{MemoryDeviceTrustStore, MemoryStateStore},
    security::{
        ChannelContext, SecureError, identity::generate_identity, service::*, simulation::connect,
    },
    simulation::SimulatedNetwork,
};
use zorvinkel_application::state::DeviceState;
use zorvinkel_core::{network::NetworkConfiguration, trust::TrustMode};
use zorvinkel_protocol::status::*;

pub fn run() -> Result<(), SecureError> {
    let client_key = IdentityHandle::from_material(generate_identity()?);
    let device_key = IdentityHandle::from_material(generate_identity()?);
    let mut trust = MemoryDeviceTrustStore::default();
    let mut store = MemoryStateStore::new(DeviceState::initial(NetworkConfiguration::Disconnected));
    let sessions = SessionRegistry::default();
    let (mut client, mut device) = connect(
        &client_key,
        &device_key,
        &mut trust,
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        Duration::ZERO,
        &sessions,
    )?;
    if request(&mut client, &mut device, &mut store, Operation::Register)?
        != ResponseBody::Registered
    {
        return Err(SecureError::Unauthorized);
    }
    println!("Encrypted in-memory simulation: first client registered; registration closed.");
    drop((client, device));
    let (mut client, mut device) = connect(
        &client_key,
        &device_key,
        &mut trust,
        TrustMode::RequireKnownKey,
        ChannelContext::Simulation,
        Duration::ZERO,
        &sessions,
    )?;
    if request(
        &mut client,
        &mut device,
        &mut store,
        Operation::Authenticate,
    )? != ResponseBody::Authenticated
    {
        return Err(SecureError::Unauthorized);
    }
    if !matches!(
        request(&mut client, &mut device, &mut store, Operation::Status)?,
        ResponseBody::Status { mode: 0, .. }
    ) {
        return Err(SecureError::InvalidMessage);
    }
    if request(
        &mut client,
        &mut device,
        &mut store,
        Operation::Capabilities,
    )? != ResponseBody::Capabilities(CAPABILITIES)
    {
        return Err(SecureError::InvalidMessage);
    }
    println!("Fresh authenticated session: status disconnected; capabilities confirmed.");
    Ok(())
}
fn request(
    client: &mut ClientSession,
    device: &mut DeviceSession,
    store: &mut MemoryStateStore,
    operation: Operation,
) -> Result<ResponseBody, SecureError> {
    let request = client.request(operation, Duration::ZERO)?;
    let reply = device.handle(
        &request,
        store,
        &mut SimulatedNetwork::default(),
        ServiceTime {
            elapsed: Duration::ZERO,
            registration_seconds: 0,
        },
    )?;
    Ok(client.receive(&reply, Duration::ZERO)?.body)
}
