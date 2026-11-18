//! Internal registration and setup transitions. Transport and OS adapters establish caller identity.
use crate::state::{ApplicationError, StateStore};
use zorvinkel_core::registry::*;

pub fn register(
    store: &mut impl StateStore,
    client: ClientKey,
    now: u64,
) -> Result<(), ApplicationError> {
    // Persist expiration even when the attempted registration is rejected.
    store
        .update(|state| Ok(state.registry.register(client, now)))?
        .map_err(Into::into)
}

pub fn grant_by_client(
    store: &mut impl StateStore,
    actor: &ClientKey,
    target: ClientKey,
    now: u64,
    expires_at: u64,
) -> Result<(), ApplicationError> {
    store.update(|state| {
        state.registry.authorize(actor)?;
        Ok(state.registry.grant(target, now, expires_at)?)
    })
}

/// Internal administration entry point. The OS adapter must verify administrator authority.
/// This function must never be dispatched from a public transport.
pub fn grant_by_local_admin(
    store: &mut impl StateStore,
    target: ClientKey,
    now: u64,
    expires_at: u64,
) -> Result<(), ApplicationError> {
    store.update(|state| Ok(state.registry.grant(target, now, expires_at)?))
}

pub fn revoke(
    store: &mut impl StateStore,
    actor: &ClientKey,
    target: &ClientKey,
) -> Result<(), ApplicationError> {
    store.update(|state| {
        state.registry.authorize(actor)?;
        Ok(state.registry.revoke(target)?)
    })
}

/// Setup command for an authenticated, registered client.
pub fn authorize_by_client(
    store: &mut impl StateStore,
    actor: &ClientKey,
    target: ClientKey,
) -> Result<(), ApplicationError> {
    store.update(|state| {
        state.registry.authorize(actor)?;
        Ok(state.registry.authorize_key(target)?)
    })
}

/// Internal setup command. The local OS adapter must verify administrator authority.
/// Never expose this command on an unauthenticated transport.
pub fn authorize_by_local_admin(
    store: &mut impl StateStore,
    target: ClientKey,
) -> Result<(), ApplicationError> {
    store.update(|state| Ok(state.registry.authorize_key(target)?))
}

/// Active transports must be terminated after a committed revocation.
pub trait SessionControl {
    fn terminate(&mut self, client: &ClientKey) -> Result<(), ApplicationError>;
}
pub fn revoke_with_sessions(
    store: &mut impl StateStore,
    sessions: &mut impl SessionControl,
    actor: &ClientKey,
    target: &ClientKey,
) -> Result<(), ApplicationError> {
    revoke(store, actor, target)?;
    sessions.terminate(target)
}
