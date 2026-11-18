use super::{PROFILE, SecureError};
use crate::identity::{IdentityHandle, IdentityMaterial};
use snow::{
    params::DHChoice,
    resolvers::{CryptoResolver, DefaultResolver},
};

pub fn generate_identity() -> Result<IdentityMaterial, SecureError> {
    let pair =
        snow::Builder::new(PROFILE.parse().map_err(|_| SecureError::Crypto)?).generate_keypair()?;
    IdentityMaterial::new(pair.public, pair.private).map_err(|_| SecureError::InvalidIdentity)
}

pub(crate) fn validate(identity: &IdentityHandle) -> Result<(), SecureError> {
    let material = &identity.material;
    if material.public.len() != 32 || material.private.len() != 32 {
        return Err(SecureError::InvalidIdentity);
    }
    let mut dh = DefaultResolver
        .resolve_dh(&DHChoice::Curve25519)
        .ok_or(SecureError::Crypto)?;
    dh.set(&material.private);
    if dh.pubkey() != material.public {
        return Err(SecureError::InvalidIdentity);
    }
    Ok(())
}
