use super::record::checksum;
use std::collections::{BTreeMap, BTreeSet};
use zorvinkel_application::state::*;
use zorvinkel_core::{network::*, registry::*};

// Storage schema only; this is not a transport encoding. Unknown schemas fail closed.
const HEADER: &[u8] = b"ZORVSTATE\x01";

pub(super) fn encode(state: &DeviceState) -> Result<Vec<u8>, StateError> {
    let mut out = Writer(HEADER.to_vec());
    match &state.registry.state {
        RegistrationState::Unclaimed => out.tag(0),
        RegistrationState::Closed => out.tag(1),
        RegistrationState::GrantOpen { client, expires_at } => {
            out.tag(2);
            out.key(client);
            out.number(*expires_at);
        }
    }
    for keys in [&state.registry.clients, &state.registry.revoked] {
        out.number(keys.len() as u64);
        for key in keys {
            out.key(key);
        }
    }
    out.configuration(&state.settings.last_accepted);
    match &state.settings.restart_policy {
        RestartPolicy::LastAccepted => out.tag(0),
        RestartPolicy::DefaultProfile(configuration) => {
            out.tag(1);
            out.configuration(configuration);
        }
    }
    out.number(state.operations.len() as u64);
    for operation in state.operations.values() {
        out.number(operation.id.0);
        out.key(&operation.owner);
        match &operation.intent {
            NetworkIntent::ConnectWifi(profile) => {
                out.tag(0);
                out.bytes(profile.as_str().as_bytes());
            }
            NetworkIntent::EnableAccessPoint(profile) => {
                out.tag(1);
                out.bytes(profile.as_str().as_bytes());
            }
            NetworkIntent::DisableAccessPoint => out.tag(2),
        }
        out.configuration(&operation.previous);
        match &operation.status {
            OperationStatus::Pending => out.tag(0),
            OperationStatus::Succeeded { actual } => {
                out.tag(1);
                out.configuration(actual);
            }
            OperationStatus::Failed {
                cause,
                actual,
                rollback_failed,
            } => {
                out.tag(2);
                out.error(*cause);
                out.configuration(actual);
                out.tag(u8::from(*rollback_failed));
            }
            OperationStatus::NeedsReconciliation { cause } => {
                out.tag(3);
                out.error(*cause);
            }
        }
    }
    let checksum = checksum(&out.0);
    out.0.extend(checksum.to_le_bytes());
    if out.0.len() > 1_048_576 || decode(&out.0)? != *state {
        return Err(StateError::Corrupt);
    }
    Ok(out.0)
}

pub(super) fn decode(bytes: &[u8]) -> Result<DeviceState, StateError> {
    let split = bytes.len().checked_sub(4).ok_or(StateError::Corrupt)?;
    let (bytes, stored) = bytes.split_at(split);
    if checksum(bytes).to_le_bytes() != stored {
        return Err(StateError::Corrupt);
    }
    decode_payload(bytes)
}

fn decode_payload(bytes: &[u8]) -> Result<DeviceState, StateError> {
    if bytes.get(..HEADER.len() - 1) != Some(&HEADER[..HEADER.len() - 1]) {
        return Err(StateError::Corrupt);
    }
    if bytes.get(HEADER.len() - 1) != Some(&1) {
        return Err(StateError::UnsupportedVersion);
    }
    let mut input = Reader(bytes.get(HEADER.len()..).ok_or(StateError::Corrupt)?);
    let state = match input.tag()? {
        0 => RegistrationState::Unclaimed,
        1 => RegistrationState::Closed,
        2 => RegistrationState::GrantOpen {
            client: input.key()?,
            expires_at: input.number()?,
        },
        _ => return Err(StateError::Corrupt),
    };
    let clients = input.keys()?;
    let revoked = input.keys()?;
    if clients.len() + revoked.len() > 256
        || !clients.is_disjoint(&revoked)
        || (state == RegistrationState::Unclaimed && (!clients.is_empty() || !revoked.is_empty()))
    {
        return Err(StateError::Corrupt);
    }
    let registry = Registry {
        state,
        clients,
        revoked,
    };
    let last_accepted = input.configuration()?;
    let restart_policy = match input.tag()? {
        0 => RestartPolicy::LastAccepted,
        1 => RestartPolicy::DefaultProfile(input.configuration()?),
        _ => return Err(StateError::Corrupt),
    };
    let mut operations = BTreeMap::new();
    for _ in 0..input.count(256)? {
        let id = OperationId(input.number()?);
        let owner = input.key()?;
        let intent = match input.tag()? {
            0 => NetworkIntent::ConnectWifi(input.profile()?),
            1 => NetworkIntent::EnableAccessPoint(input.profile()?),
            2 => NetworkIntent::DisableAccessPoint,
            _ => return Err(StateError::Corrupt),
        };
        let previous = input.configuration()?;
        let status = match input.tag()? {
            0 => OperationStatus::Pending,
            1 => OperationStatus::Succeeded {
                actual: input.configuration()?,
            },
            2 => OperationStatus::Failed {
                cause: input.error()?,
                actual: input.configuration()?,
                rollback_failed: match input.tag()? {
                    0 => false,
                    1 => true,
                    _ => return Err(StateError::Corrupt),
                },
            },
            3 => OperationStatus::NeedsReconciliation {
                cause: input.error()?,
            },
            _ => return Err(StateError::Corrupt),
        };
        if operations
            .insert(
                id,
                NetworkOperation {
                    id,
                    owner,
                    intent,
                    previous,
                    status,
                },
            )
            .is_some()
        {
            return Err(StateError::Corrupt);
        }
    }
    if !input.0.is_empty()
        || operations
            .values()
            .filter(|operation| operation.status.is_pending())
            .count()
            > 1
    {
        return Err(StateError::Corrupt);
    }
    Ok(DeviceState {
        registry,
        settings: NetworkSettings {
            last_accepted,
            restart_policy,
        },
        operations,
    })
}

struct Writer(Vec<u8>);
impl Writer {
    fn tag(&mut self, value: u8) {
        self.0.push(value);
    }
    fn number(&mut self, value: u64) {
        self.0.extend(value.to_le_bytes());
    }
    fn bytes(&mut self, bytes: &[u8]) {
        self.number(bytes.len() as u64);
        self.0.extend(bytes);
    }
    fn key(&mut self, key: &ClientKey) {
        self.bytes(key.as_bytes());
    }
    fn configuration(&mut self, configuration: &NetworkConfiguration) {
        match configuration {
            NetworkConfiguration::Disconnected => self.tag(0),
            NetworkConfiguration::Wifi(profile) => {
                self.tag(1);
                self.bytes(profile.as_str().as_bytes());
            }
            NetworkConfiguration::AccessPoint(profile) => {
                self.tag(2);
                self.bytes(profile.as_str().as_bytes());
            }
        }
    }
    fn error(&mut self, error: NetworkError) {
        self.tag(match error {
            NetworkError::InvalidInput => 0,
            NetworkError::Unsupported => 1,
            NetworkError::Unavailable => 2,
            NetworkError::TimedOut => 3,
            NetworkError::Rejected => 4,
            NetworkError::Interrupted => 5,
        });
    }
}
struct Reader<'a>(&'a [u8]);
impl Reader<'_> {
    fn take(&mut self, length: usize) -> Result<&[u8], StateError> {
        if length > self.0.len() {
            return Err(StateError::Corrupt);
        }
        let (head, tail) = self.0.split_at(length);
        self.0 = tail;
        Ok(head)
    }
    fn tag(&mut self) -> Result<u8, StateError> {
        Ok(self.take(1)?[0])
    }
    fn number(&mut self) -> Result<u64, StateError> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().map_err(|_| StateError::Corrupt)?,
        ))
    }
    fn count(&mut self, limit: usize) -> Result<usize, StateError> {
        let count = usize::try_from(self.number()?).map_err(|_| StateError::Corrupt)?;
        if count > limit {
            return Err(StateError::Corrupt);
        }
        Ok(count)
    }
    fn key(&mut self) -> Result<ClientKey, StateError> {
        let length = self.count(1024)?;
        ClientKey::new(self.take(length)?.to_vec()).map_err(|_| StateError::Corrupt)
    }
    fn keys(&mut self) -> Result<BTreeSet<ClientKey>, StateError> {
        let mut keys = BTreeSet::new();
        for _ in 0..self.count(256)? {
            if !keys.insert(self.key()?) {
                return Err(StateError::Corrupt);
            }
        }
        Ok(keys)
    }
    fn profile(&mut self) -> Result<ProfileId, StateError> {
        let length = self.count(256)?;
        let value =
            String::from_utf8(self.take(length)?.to_vec()).map_err(|_| StateError::Corrupt)?;
        ProfileId::new(value).map_err(|_| StateError::Corrupt)
    }
    fn configuration(&mut self) -> Result<NetworkConfiguration, StateError> {
        match self.tag()? {
            0 => Ok(NetworkConfiguration::Disconnected),
            1 => Ok(NetworkConfiguration::Wifi(self.profile()?)),
            2 => Ok(NetworkConfiguration::AccessPoint(self.profile()?)),
            _ => Err(StateError::Corrupt),
        }
    }
    fn error(&mut self) -> Result<NetworkError, StateError> {
        match self.tag()? {
            0 => Ok(NetworkError::InvalidInput),
            1 => Ok(NetworkError::Unsupported),
            2 => Ok(NetworkError::Unavailable),
            3 => Ok(NetworkError::TimedOut),
            4 => Ok(NetworkError::Rejected),
            5 => Ok(NetworkError::Interrupted),
            _ => Err(StateError::Corrupt),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn damaged_and_truncated_snapshots_fail_closed() {
        let encoded = encode(&DeviceState::initial(NetworkConfiguration::Disconnected)).unwrap();
        for length in 0..encoded.len() {
            assert!(decode(&encoded[..length]).is_err());
        }
        for index in 0..encoded.len() {
            let mut damaged = encoded.clone();
            damaged[index] ^= 1;
            assert!(decode(&damaged).is_err());
        }
    }
    #[test]
    fn unknown_schema_is_rejected() {
        let mut encoded =
            encode(&DeviceState::initial(NetworkConfiguration::Disconnected)).unwrap();
        encoded[HEADER.len() - 1] = 2;
        let end = encoded.len() - 4;
        let crc = checksum(&encoded[..end]);
        encoded[end..].copy_from_slice(&crc.to_le_bytes());
        assert_eq!(decode(&encoded), Err(StateError::UnsupportedVersion));
    }
}
