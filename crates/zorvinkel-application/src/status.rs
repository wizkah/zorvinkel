use crate::{
    device::NetworkStatusSource,
    state::{ApplicationError, StateStore},
};
use zorvinkel_core::{NetworkStatus, registry::ClientKey};

/// The security adapter must supply the identity proven by the current handshake.
pub fn authenticate(
    store: &mut impl StateStore,
    client: &ClientKey,
) -> Result<(), ApplicationError> {
    store.read()?.registry.authorize(client)?;
    Ok(())
}
pub fn read_network_status(
    store: &mut impl StateStore,
    network: &mut impl NetworkStatusSource,
    client: &ClientKey,
) -> Result<NetworkStatus, ApplicationError> {
    authenticate(store, client)?;
    network
        .read_status()
        .map_err(|_| ApplicationError::Network(zorvinkel_core::network::NetworkError::Unavailable))
}
