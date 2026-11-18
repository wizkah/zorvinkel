use zeroize::Zeroize;
use zorvinkel_application::state::StateError;

/// Deliberately has no Debug, Clone, serialization or private-key export API.
///
/// ```compile_fail
/// use zorvinkel_adapters::identity::IdentityMaterial;
/// let material = IdentityMaterial::new(vec![1; 32], vec![2; 32]).unwrap();
/// let _private_key = material.private;
/// ```
pub struct IdentityMaterial {
    pub(crate) public: Vec<u8>,
    pub(crate) private: Vec<u8>,
}
impl IdentityMaterial {
    /// The security adapter must supply an internally validated key pair.
    pub fn new(public: Vec<u8>, private: Vec<u8>) -> Result<Self, StateError> {
        if public.is_empty() || public.len() > 1024 || private.is_empty() || private.len() > 4096 {
            return Err(StateError::Corrupt);
        }
        Ok(Self { public, private })
    }
}

/// Opaque owner of the key material. Only security adapters in this crate may use it.
pub struct IdentityHandle {
    pub(crate) material: IdentityMaterial,
}

impl Drop for IdentityMaterial {
    fn drop(&mut self) {
        self.private.zeroize();
    }
}
impl IdentityHandle {
    pub fn public_key(&self) -> &[u8] {
        &self.material.public
    }
    pub fn from_material(material: IdentityMaterial) -> Self {
        Self { material }
    }
}
