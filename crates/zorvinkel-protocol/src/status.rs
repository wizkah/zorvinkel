//! Versioned, bounded messages for the secured status channel.
pub const MAX_RECORD: usize = 4096;
pub const MAX_PLAINTEXT: usize = MAX_RECORD - 4 - 16;
pub const CAPABILITIES: u32 = 3;
const HEADER: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodecError {
    Malformed,
    Version,
    TooLarge,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Register,
    Authenticate,
    Status,
    Capabilities,
    WifiScan,
    WifiConnect,
    WifiPoll,
    WifiPersistence,
    Power,
    ApRead,
    ApWrite,
    ApEnable,
    ApDisable,
    SystemInfo,
    Services,
    ServiceAction,
    /// Issues the one-time proof that ties an IP session to this session (Architecture §5).
    CorrelationIssue,
    /// Redeems that proof over the IP session; only the same client can redeem it.
    CorrelationRedeem,
    /// Reads the restart option of the device.
    RestartPolicyRead,
    /// Chooses the restart option; completion is followed like other network tasks.
    RestartPolicySet,
    /// Opens the terminal tunnel to the device's local `sshd` (ADR-031).
    TunnelOpen,
    Unknown(u16),
}
impl Operation {
    pub fn code(self) -> u16 {
        match self {
            Self::Register => 1,
            Self::Authenticate => 2,
            Self::Status => 3,
            Self::Capabilities => 4,
            Self::WifiScan => 5,
            Self::WifiConnect => 6,
            Self::WifiPoll => 7,
            Self::WifiPersistence => 8,
            Self::Power => 9,
            Self::ApRead => 10,
            Self::ApWrite => 11,
            Self::ApEnable => 12,
            Self::ApDisable => 13,
            Self::SystemInfo => 14,
            Self::Services => 15,
            Self::ServiceAction => 16,
            Self::CorrelationIssue => 17,
            Self::CorrelationRedeem => 18,
            Self::RestartPolicyRead => 19,
            Self::RestartPolicySet => 20,
            Self::TunnelOpen => 21,
            Self::Unknown(code) => code,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub operation: Operation,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteError {
    Unauthorized,
    RegistrationClosed,
    Unsupported,
    Unavailable,
    Busy,
    Conflict,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResponseBody {
    Registered,
    Authenticated,
    Status { mode: u8, ssid: String },
    Capabilities(u32),
    WifiScanStarted(u64),
    WifiConnectStarted(u64),
    WifiTask(crate::wifi::WifiTask),
    WifiPersistence(bool),
    PowerAccepted(crate::power::PowerCommand),
    ApCredentials(Option<crate::ap::ApCredentials>),
    ApStored,
    ApStarted(u64),
    SystemInfo(crate::system::SystemInfo),
    Services(Vec<crate::services::ServiceEntry>),
    ServiceActionAccepted(crate::services::ServiceAction, String),
    Correlation(crate::ip::CorrelationToken),
    Correlated,
    RestartPolicy(crate::restart::RestartState),
    RestartPolicyStarted(u64),
    TunnelOpened(crate::tunnel::OpenResponse),
    Error(RemoteError),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response {
    pub id: u64,
    pub body: ResponseBody,
}

pub fn encode_request(request: Request) -> Result<Vec<u8>, CodecError> {
    encode_request_payload(request, &[])
}
pub fn encode_request_payload(request: Request, payload: &[u8]) -> Result<Vec<u8>, CodecError> {
    if matches!(request.operation, Operation::Unknown(code) if (1..=21).contains(&code) || code >= 0x8000)
    {
        return Err(CodecError::Malformed);
    }
    validate_request_payload(request.operation, payload)?;
    envelope(request.id, request.operation.code(), payload)
}
fn validate_request_payload(operation: Operation, payload: &[u8]) -> Result<(), CodecError> {
    match operation {
        Operation::WifiScan | Operation::WifiPoll => {
            crate::wifi::read_id(payload)?;
        }
        Operation::WifiConnect => {
            crate::wifi::read_connect(payload)?;
        }
        Operation::WifiPersistence => {
            crate::wifi::read_persistence(payload)?;
        }
        Operation::Power => {
            crate::power::read_power(payload)?;
        }
        Operation::ApWrite => {
            crate::ap::read_credentials(payload)?;
        }
        Operation::ApEnable | Operation::ApDisable => {
            crate::ap::read_id(payload)?;
        }
        Operation::ServiceAction => {
            crate::services::read_action(payload)?;
        }
        Operation::CorrelationRedeem => {
            crate::ip::read_correlation(payload)?;
        }
        Operation::RestartPolicySet => {
            crate::restart::read_set(payload)?;
        }
        Operation::TunnelOpen => {
            crate::tunnel::OpenRequest::parse(payload)?;
        }
        _ if !payload.is_empty() => return Err(CodecError::Malformed),
        _ => (),
    }
    Ok(())
}
pub fn request_payload(bytes: &[u8]) -> Result<&[u8], CodecError> {
    decode_request(bytes)?;
    Ok(&bytes[HEADER..])
}
pub fn decode_request(bytes: &[u8]) -> Result<Request, CodecError> {
    let (id, code, payload) = decode(bytes)?;
    if code >= 0x8000 {
        return Err(CodecError::Malformed);
    }
    let operation = match code {
        1 => Operation::Register,
        2 => Operation::Authenticate,
        3 => Operation::Status,
        4 => Operation::Capabilities,
        5 => Operation::WifiScan,
        6 => Operation::WifiConnect,
        7 => Operation::WifiPoll,
        8 => Operation::WifiPersistence,
        9 => Operation::Power,
        10 => Operation::ApRead,
        11 => Operation::ApWrite,
        12 => Operation::ApEnable,
        13 => Operation::ApDisable,
        14 => Operation::SystemInfo,
        15 => Operation::Services,
        16 => Operation::ServiceAction,
        17 => Operation::CorrelationIssue,
        18 => Operation::CorrelationRedeem,
        19 => Operation::RestartPolicyRead,
        20 => Operation::RestartPolicySet,
        21 => Operation::TunnelOpen,
        other => Operation::Unknown(other),
    };
    validate_request_payload(operation, payload)?;
    Ok(Request { id, operation })
}
pub fn encode_response(response: &Response) -> Result<Vec<u8>, CodecError> {
    let (code, payload) = match &response.body {
        ResponseBody::Registered => (0x8001, Vec::new()),
        ResponseBody::Authenticated => (0x8002, Vec::new()),
        ResponseBody::Status { mode, ssid } => {
            validate_status(*mode, ssid)?;
            let mut payload = vec![*mode, ssid.len() as u8];
            payload.extend(ssid.as_bytes());
            (0x8003, payload)
        }
        ResponseBody::Capabilities(bits) => (0x8004, bits.to_le_bytes().to_vec()),
        ResponseBody::WifiScanStarted(id) => (0x8005, crate::wifi::id_payload(*id)?),
        ResponseBody::WifiConnectStarted(id) => (0x8006, crate::wifi::id_payload(*id)?),
        ResponseBody::WifiTask(task) => (0x8007, crate::wifi::encode_task(task)?),
        ResponseBody::WifiPersistence(enabled) => (0x8008, vec![u8::from(*enabled)]),
        ResponseBody::PowerAccepted(command) => (0x8009, crate::power::power_payload(*command)),
        ResponseBody::ApCredentials(stored) => (0x800a, crate::ap::encode_stored(stored)?),
        ResponseBody::ApStored => (0x800b, Vec::new()),
        ResponseBody::ApStarted(id) => (0x800c, crate::ap::id_payload(*id)?),
        ResponseBody::SystemInfo(info) => (0x800d, crate::system::encode(info)?),
        ResponseBody::Services(services) => (0x800e, crate::services::encode(services)?),
        ResponseBody::ServiceActionAccepted(action, name) => {
            (0x800f, crate::services::action_payload(*action, name)?)
        }
        ResponseBody::Correlation(token) => (0x8010, token.0.to_vec()),
        ResponseBody::Correlated => (0x8011, Vec::new()),
        ResponseBody::RestartPolicy(state) => (0x8012, crate::restart::state_payload(*state)),
        ResponseBody::RestartPolicyStarted(id) => (0x8013, crate::wifi::id_payload(*id)?),
        ResponseBody::TunnelOpened(response) => (0x8014, response.to_bytes()),
        ResponseBody::Error(error) => (
            0xffff,
            vec![match error {
                RemoteError::Unauthorized => 1,
                RemoteError::RegistrationClosed => 2,
                RemoteError::Unsupported => 3,
                RemoteError::Unavailable => 4,
                RemoteError::Busy => 5,
                RemoteError::Conflict => 6,
            }],
        ),
    };
    envelope(response.id, code, &payload)
}
pub fn decode_response(bytes: &[u8]) -> Result<Response, CodecError> {
    let (id, code, payload) = decode(bytes)?;
    let body = match code {
        0x8001 if payload.is_empty() => ResponseBody::Registered,
        0x8002 if payload.is_empty() => ResponseBody::Authenticated,
        0x8003 if payload.len() >= 2 && payload.len() == 2 + usize::from(payload[1]) => {
            let ssid = std::str::from_utf8(&payload[2..])
                .map_err(|_| CodecError::Malformed)?
                .to_owned();
            validate_status(payload[0], &ssid)?;
            ResponseBody::Status {
                mode: payload[0],
                ssid,
            }
        }
        0x8004 if payload.len() == 4 => ResponseBody::Capabilities(u32::from_le_bytes(
            payload.try_into().map_err(|_| CodecError::Malformed)?,
        )),
        0x8005 => ResponseBody::WifiScanStarted(crate::wifi::read_id(payload)?),
        0x8006 => ResponseBody::WifiConnectStarted(crate::wifi::read_id(payload)?),
        0x8007 => ResponseBody::WifiTask(crate::wifi::decode_task(payload)?),
        0x8008 => ResponseBody::WifiPersistence(match payload {
            [0] => false,
            [1] => true,
            _ => return Err(CodecError::Malformed),
        }),
        0x8009 => ResponseBody::PowerAccepted(crate::power::read_power(payload)?),
        0x800a => ResponseBody::ApCredentials(crate::ap::decode_stored(payload)?),
        0x800b if payload.is_empty() => ResponseBody::ApStored,
        0x800c => ResponseBody::ApStarted(crate::ap::read_id(payload)?),
        0x800d => ResponseBody::SystemInfo(crate::system::decode(payload)?),
        0x800e => ResponseBody::Services(crate::services::decode(payload)?),
        0x800f => {
            let (action, name) = crate::services::read_action(payload)?;
            ResponseBody::ServiceActionAccepted(action, name)
        }
        0x8010 => ResponseBody::Correlation(crate::ip::read_correlation(payload)?),
        0x8011 if payload.is_empty() => ResponseBody::Correlated,
        0x8012 => ResponseBody::RestartPolicy(crate::restart::read_state(payload)?),
        0x8013 => ResponseBody::RestartPolicyStarted(crate::wifi::read_id(payload)?),
        0x8014 => ResponseBody::TunnelOpened(crate::tunnel::OpenResponse::parse(payload)?),
        0xffff if payload.len() == 1 => ResponseBody::Error(match payload[0] {
            1 => RemoteError::Unauthorized,
            2 => RemoteError::RegistrationClosed,
            3 => RemoteError::Unsupported,
            4 => RemoteError::Unavailable,
            5 => RemoteError::Busy,
            6 => RemoteError::Conflict,
            _ => return Err(CodecError::Malformed),
        }),
        _ => return Err(CodecError::Malformed),
    };
    Ok(Response { id, body })
}
fn validate_status(mode: u8, ssid: &str) -> Result<(), CodecError> {
    if mode > 2
        || ssid.len() > 32
        || ssid.chars().any(char::is_control)
        || (mode == 0) != ssid.is_empty()
    {
        return Err(CodecError::Malformed);
    }
    Ok(())
}
fn envelope(id: u64, code: u16, payload: &[u8]) -> Result<Vec<u8>, CodecError> {
    if id == 0 {
        return Err(CodecError::Malformed);
    }
    if HEADER + payload.len() > MAX_PLAINTEXT {
        return Err(CodecError::TooLarge);
    }
    let mut bytes = b"ZV\x01\x00".to_vec();
    bytes.extend(id.to_le_bytes());
    bytes.extend(code.to_le_bytes());
    bytes.extend((payload.len() as u16).to_le_bytes());
    bytes.extend(payload);
    Ok(bytes)
}
fn decode(bytes: &[u8]) -> Result<(u64, u16, &[u8]), CodecError> {
    if bytes.len() < HEADER {
        return Err(CodecError::Malformed);
    }
    if bytes.len() > MAX_PLAINTEXT {
        return Err(CodecError::TooLarge);
    }
    if &bytes[..2] != b"ZV" {
        return Err(CodecError::Malformed);
    }
    if bytes[2..4] != [1, 0] {
        return Err(CodecError::Version);
    }
    let id = u64::from_le_bytes(bytes[4..12].try_into().map_err(|_| CodecError::Malformed)?);
    let code = u16::from_le_bytes(
        bytes[12..14]
            .try_into()
            .map_err(|_| CodecError::Malformed)?,
    );
    let length = u16::from_le_bytes(
        bytes[14..16]
            .try_into()
            .map_err(|_| CodecError::Malformed)?,
    ) as usize;
    if id == 0 || bytes.len() != HEADER + length {
        return Err(CodecError::Malformed);
    }
    Ok((id, code, &bytes[HEADER..]))
}
pub fn wrap_record(payload: &[u8]) -> Result<Vec<u8>, CodecError> {
    if payload.is_empty() {
        return Err(CodecError::Malformed);
    }
    if payload.len() > MAX_RECORD - 4 {
        return Err(CodecError::TooLarge);
    }
    let mut bytes = (payload.len() as u32).to_be_bytes().to_vec();
    bytes.extend(payload);
    Ok(bytes)
}
pub fn record_payload(record: &[u8]) -> Result<&[u8], CodecError> {
    if record.len() < 5 {
        return Err(CodecError::Malformed);
    }
    if record.len() > MAX_RECORD {
        return Err(CodecError::TooLarge);
    }
    let length =
        u32::from_be_bytes(record[..4].try_into().map_err(|_| CodecError::Malformed)?) as usize;
    if length != record.len() - 4 {
        return Err(CodecError::Malformed);
    }
    Ok(&record[4..])
}

impl std::fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("invalid protocol message")
    }
}
impl std::error::Error for CodecError {}
