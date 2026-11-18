use crate::registry::ClientKey;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileId(String);
impl ProfileId {
    pub fn new(value: String) -> Result<Self, NetworkError> {
        if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err(NetworkError::InvalidInput);
        }
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkConfiguration {
    Disconnected,
    Wifi(ProfileId),
    AccessPoint(ProfileId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetworkIntent {
    ConnectWifi(ProfileId),
    EnableAccessPoint(ProfileId),
    DisableAccessPoint,
}

impl NetworkIntent {
    pub fn target(&self, current: &NetworkConfiguration) -> NetworkConfiguration {
        match self {
            Self::ConnectWifi(profile) => NetworkConfiguration::Wifi(profile.clone()),
            Self::EnableAccessPoint(profile) => NetworkConfiguration::AccessPoint(profile.clone()),
            Self::DisableAccessPoint => match current {
                NetworkConfiguration::AccessPoint(_) => NetworkConfiguration::Disconnected,
                configuration => configuration.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum RestartPolicy {
    #[default]
    LastAccepted,
    DefaultProfile(NetworkConfiguration),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkSettings {
    pub last_accepted: NetworkConfiguration,
    pub restart_policy: RestartPolicy,
}
impl NetworkSettings {
    pub fn restart_configuration(&self) -> &NetworkConfiguration {
        match &self.restart_policy {
            RestartPolicy::LastAccepted => &self.last_accepted,
            RestartPolicy::DefaultProfile(configuration) => configuration,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct OperationId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkError {
    InvalidInput,
    Unsupported,
    Unavailable,
    TimedOut,
    Rejected,
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationStatus {
    Pending,
    Succeeded {
        actual: NetworkConfiguration,
    },
    Failed {
        cause: NetworkError,
        actual: NetworkConfiguration,
        rollback_failed: bool,
    },
    NeedsReconciliation {
        cause: NetworkError,
    },
}
impl OperationStatus {
    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending | Self::NeedsReconciliation { .. })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkOperation {
    pub id: OperationId,
    pub owner: ClientKey,
    pub intent: NetworkIntent,
    pub previous: NetworkConfiguration,
    pub status: OperationStatus,
}
