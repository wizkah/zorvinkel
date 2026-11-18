//! Services the device operator configured for display (ADR-021). Read-only; needs an authorized session.
use crate::{
    status::CodecError,
    system::{Reader, put_text},
};

/// The device lists configured services with version, state and optional web page.
pub const CAP_SERVICES: u32 = 256;

/// The device can start, stop and restart services whose configuration allows it.
pub const CAP_SERVICE_ACTIONS: u32 = 512;

pub const MAX_SERVICES: usize = 16;
pub const MAX_NAME: usize = 32;
pub const MAX_VERSION: usize = 32;
pub const MAX_PATH: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
}
impl ServiceAction {
    fn bit(self) -> u8 {
        match self {
            Self::Start => 1,
            Self::Stop => 2,
            Self::Restart => 4,
        }
    }
}
const ALL_ACTIONS: [ServiceAction; 3] = [
    ServiceAction::Start,
    ServiceAction::Stop,
    ServiceAction::Restart,
];

/// Request and acknowledgement payload: action code (1 start, 2 stop, 3 restart) and service name.
pub fn action_payload(action: ServiceAction, name: &str) -> Result<Vec<u8>, CodecError> {
    if !text_valid(name, MAX_NAME) {
        return Err(CodecError::Malformed);
    }
    let mut bytes = vec![match action {
        ServiceAction::Start => 1,
        ServiceAction::Stop => 2,
        ServiceAction::Restart => 3,
    }];
    put_text(&mut bytes, name);
    Ok(bytes)
}
pub fn read_action(payload: &[u8]) -> Result<(ServiceAction, String), CodecError> {
    let mut reader = Reader(payload);
    let action = match reader.take(1)?[0] {
        1 => ServiceAction::Start,
        2 => ServiceAction::Stop,
        3 => ServiceAction::Restart,
        _ => return Err(CodecError::Malformed),
    };
    let name = reader.text()?;
    if !reader.0.is_empty() || !text_valid(&name, MAX_NAME) {
        return Err(CodecError::Malformed);
    }
    Ok((action, name))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceState {
    Unknown,
    Active,
    Activating,
    Inactive,
    Failed,
}
impl ServiceState {
    fn code(self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::Active => 1,
            Self::Activating => 2,
            Self::Inactive => 3,
            Self::Failed => 4,
        }
    }
    fn from_code(code: u8) -> Result<Self, CodecError> {
        Ok(match code {
            0 => Self::Unknown,
            1 => Self::Active,
            2 => Self::Activating,
            3 => Self::Inactive,
            4 => Self::Failed,
            _ => return Err(CodecError::Malformed),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebScheme {
    Http,
    Https,
}

/// Web page of a service. The host is not part of it: the client takes a device address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebLink {
    /// Caption of the link; the client shows its own text when absent.
    pub label: Option<String>,
    pub scheme: WebScheme,
    pub port: u16,
    /// Starts with `/`; printable ASCII without spaces.
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceEntry {
    /// Tab of the device page the service is listed under.
    pub group: String,
    pub name: String,
    pub version: Option<String>,
    pub state: ServiceState,
    /// Actions the operator allowed; empty when the device cannot perform them.
    pub actions: Vec<ServiceAction>,
    pub web: Option<WebLink>,
}

fn text_valid(text: &str, max: usize) -> bool {
    !text.is_empty() && text.len() <= max && !text.chars().any(char::is_control)
}
fn path_valid(path: &str) -> bool {
    path.len() <= MAX_PATH && path.starts_with('/') && path.chars().all(|c| c.is_ascii_graphic())
}
fn validate(services: &[ServiceEntry]) -> Result<(), CodecError> {
    let entry_valid = |entry: &ServiceEntry| {
        text_valid(&entry.group, MAX_NAME)
            && text_valid(&entry.name, MAX_NAME)
            && entry
                .version
                .as_deref()
                .is_none_or(|version| text_valid(version, MAX_VERSION))
            && entry.web.as_ref().is_none_or(|web| {
                web.label
                    .as_deref()
                    .is_none_or(|label| text_valid(label, MAX_NAME))
                    && web.port != 0
                    && path_valid(&web.path)
            })
    };
    let actions_valid = |entry: &ServiceEntry| {
        let mut seen = Vec::new();
        entry.actions.iter().all(|action| {
            let fresh = !seen.contains(action);
            seen.push(*action);
            fresh
        })
    };
    if services.len() > MAX_SERVICES || !services.iter().all(|e| entry_valid(e) && actions_valid(e))
    {
        return Err(CodecError::Malformed);
    }
    Ok(())
}

pub fn encode(services: &[ServiceEntry]) -> Result<Vec<u8>, CodecError> {
    validate(services)?;
    let mut bytes = vec![services.len() as u8];
    for entry in services {
        put_text(&mut bytes, &entry.group);
        put_text(&mut bytes, &entry.name);
        put_text(&mut bytes, entry.version.as_deref().unwrap_or(""));
        bytes.push(entry.state.code());
        bytes.push(
            entry
                .actions
                .iter()
                .fold(0, |mask, action| mask | action.bit()),
        );
        match &entry.web {
            None => bytes.push(0),
            Some(web) => {
                bytes.push(match web.scheme {
                    WebScheme::Http => 1,
                    WebScheme::Https => 2,
                });
                bytes.extend(web.port.to_le_bytes());
                put_text(&mut bytes, &web.path);
                put_text(&mut bytes, web.label.as_deref().unwrap_or(""));
            }
        }
    }
    Ok(bytes)
}

pub fn decode(payload: &[u8]) -> Result<Vec<ServiceEntry>, CodecError> {
    let mut reader = Reader(payload);
    let count = usize::from(reader.take(1)?[0]);
    if count > MAX_SERVICES {
        return Err(CodecError::Malformed);
    }
    let mut services = Vec::with_capacity(count);
    for _ in 0..count {
        let group = reader.text()?;
        let name = reader.text()?;
        let version = Some(reader.text()?).filter(|version| !version.is_empty());
        let state = ServiceState::from_code(reader.take(1)?[0])?;
        let mask = reader.take(1)?[0];
        if mask > 7 {
            return Err(CodecError::Malformed);
        }
        let actions = ALL_ACTIONS
            .into_iter()
            .filter(|action| mask & action.bit() != 0)
            .collect();
        let web = match reader.take(1)?[0] {
            0 => None,
            scheme @ (1 | 2) => Some(WebLink {
                scheme: if scheme == 1 {
                    WebScheme::Http
                } else {
                    WebScheme::Https
                },
                port: u16::from_le_bytes(reader.array()?),
                path: reader.text()?,
                label: Some(reader.text()?).filter(|label| !label.is_empty()),
            }),
            _ => return Err(CodecError::Malformed),
        };
        services.push(ServiceEntry {
            group,
            name,
            version,
            state,
            actions,
            web,
        });
    }
    if !reader.0.is_empty() {
        return Err(CodecError::Malformed);
    }
    validate(&services)?;
    Ok(services)
}
