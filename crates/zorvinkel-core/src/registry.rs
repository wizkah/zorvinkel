use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClientKey(Vec<u8>);

impl ClientKey {
    pub fn new(bytes: Vec<u8>) -> Result<Self, RegistryError> {
        if bytes.is_empty() || bytes.len() > 1024 {
            return Err(RegistryError::InvalidKey);
        }
        Ok(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistrationState {
    Unclaimed,
    Closed,
    GrantOpen { client: ClientKey, expires_at: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registry {
    pub state: RegistrationState,
    pub clients: BTreeSet<ClientKey>,
    pub revoked: BTreeSet<ClientKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryError {
    InvalidKey,
    Closed,
    Revoked,
    Unauthorized,
    InvalidGrant,
    Capacity,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            state: RegistrationState::Unclaimed,
            clients: BTreeSet::new(),
            revoked: BTreeSet::new(),
        }
    }
}

impl Registry {
    pub fn authorize(&self, client: &ClientKey) -> Result<(), RegistryError> {
        if self.clients.contains(client) {
            Ok(())
        } else {
            Err(RegistryError::Unauthorized)
        }
    }

    pub fn expire_grant(&mut self, now: u64) {
        if matches!(self.state, RegistrationState::GrantOpen { expires_at, .. } if now >= expires_at)
        {
            self.state = RegistrationState::Closed;
        }
    }

    pub fn register(&mut self, client: ClientKey, now: u64) -> Result<(), RegistryError> {
        self.expire_grant(now);
        if self.revoked.contains(&client) {
            return Err(RegistryError::Revoked);
        }
        if self.clients.contains(&client) {
            return Ok(());
        }
        if self.clients.len() + self.revoked.len() >= 256 {
            return Err(RegistryError::Capacity);
        }
        let permitted = match &self.state {
            RegistrationState::Unclaimed => true,
            RegistrationState::Closed => false,
            RegistrationState::GrantOpen {
                client: target,
                expires_at,
            } => target == &client && now < *expires_at,
        };
        if !permitted {
            return Err(RegistryError::Closed);
        }
        self.clients.insert(client);
        self.state = RegistrationState::Closed;
        Ok(())
    }

    /// The caller must establish local administrator authority or authorize a registered client.
    pub fn grant(
        &mut self,
        client: ClientKey,
        now: u64,
        expires_at: u64,
    ) -> Result<(), RegistryError> {
        if self.revoked.contains(&client) {
            return Err(RegistryError::Revoked);
        }
        if expires_at <= now || self.clients.contains(&client) {
            return Err(RegistryError::InvalidGrant);
        }
        self.state = RegistrationState::GrantOpen { client, expires_at };
        Ok(())
    }

    /// Setup authorization requires a registered client or verified local administrator.
    pub fn authorize_key(&mut self, client: ClientKey) -> Result<(), RegistryError> {
        if self.revoked.contains(&client) {
            return Err(RegistryError::Revoked);
        }
        if !self.clients.contains(&client) && self.clients.len() + self.revoked.len() >= 256 {
            return Err(RegistryError::Capacity);
        }
        self.clients.insert(client);
        self.state = RegistrationState::Closed;
        Ok(())
    }

    pub fn revoke(&mut self, client: &ClientKey) -> Result<(), RegistryError> {
        if !self.clients.remove(client) {
            return Err(RegistryError::Unauthorized);
        }
        self.revoked.insert(client.clone());
        self.state = RegistrationState::Closed;
        Ok(())
    }
}
