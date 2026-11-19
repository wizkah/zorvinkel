use crate::{Connector, Credentials, TerminalSize};
use russh::{
    ChannelMsg, Disconnect,
    client::{self, Handle},
    keys::{HashAlg, PublicKeyOrCertificate},
};
use std::{fmt, future::Future, sync::Arc, time::Duration};
use tokio::{net::lookup_host, sync::mpsc, time::timeout};

/// Attaches to the session `zorvinkel`, creating it when missing (ADR-030).
pub(crate) const REMOTE_COMMAND: &str = "tmux new-session -A -s zorvinkel";
const TERMINAL_TYPE: &str = "xterm-256color";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(5);
const KEEPALIVE_MAX_MISSED: usize = 3;
const QUEUE_LENGTH: usize = 64;

#[derive(Debug)]
pub enum ConnectError {
    /// Address, connection, timeout or an interrupted handshake.
    Network(String),
    HostKeyRejected,
    AuthenticationFailed,
    /// The device refused the terminal session (for example no pseudo-terminal).
    Refused(String),
}
impl ConnectError {
    /// Only transport problems are worth another attempt; wrong passwords or changed host keys
    /// need the user.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Network(_))
    }
}
impl fmt::Display for ConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Network(reason) => write!(formatter, "connection failed: {reason}"),
            Self::HostKeyRejected => formatter.write_str("host key rejected"),
            Self::AuthenticationFailed => formatter.write_str("authentication failed"),
            Self::Refused(reason) => write!(formatter, "terminal refused: {reason}"),
        }
    }
}
impl std::error::Error for ConnectError {}

/// Decides whether to trust a host key, given its SHA-256 fingerprint (`SHA256:…`). The caller
/// owns the trust store and any prompt (ADR-030: binding to the device identity is still open).
pub trait HostKeyVerifier: Send + Sync {
    fn verify(&self, fingerprint: &str) -> bool;
}
impl<F: Fn(&str) -> bool + Send + Sync> HostKeyVerifier for F {
    fn verify(&self, fingerprint: &str) -> bool {
        self(fingerprint)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    /// The remote program ended with this exit status (also when the user leaves `tmux`).
    Exited(u32),
    /// The connection broke without an exit status.
    Lost,
    /// Closed locally.
    Closed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TerminalEvent {
    Output(Vec<u8>),
    Ended(EndReason),
}

#[derive(Debug, PartialEq, Eq)]
pub struct SessionClosed;

pub(crate) enum Command {
    Input(Vec<u8>),
    Resize(TerminalSize),
    Close,
}

/// One live SSH connection with a pseudo-terminal. Dropping it closes the connection.
pub struct TerminalSession {
    commands: mpsc::Sender<Command>,
    events: mpsc::Receiver<TerminalEvent>,
}
impl TerminalSession {
    pub(crate) fn from_channels(
        commands: mpsc::Sender<Command>,
        events: mpsc::Receiver<TerminalEvent>,
    ) -> Self {
        Self { commands, events }
    }
    pub async fn input(&self, bytes: Vec<u8>) -> Result<(), SessionClosed> {
        self.send(Command::Input(bytes)).await
    }
    pub async fn resize(&self, size: TerminalSize) -> Result<(), SessionClosed> {
        self.send(Command::Resize(size)).await
    }
    pub async fn close(&self) {
        let _ = self.send(Command::Close).await;
    }
    /// `None` once the session ended and `Ended` was delivered.
    pub async fn next_event(&mut self) -> Option<TerminalEvent> {
        self.events.recv().await
    }
    async fn send(&self, command: Command) -> Result<(), SessionClosed> {
        self.commands.send(command).await.map_err(|_| SessionClosed)
    }
}

/// Connects to the device's `sshd` with a password (ADR-030).
pub struct SshConnector {
    address: String,
    credentials: Credentials,
    verifier: Arc<dyn HostKeyVerifier>,
}
impl SshConnector {
    /// `address` is `host:port`.
    pub fn new(
        address: impl Into<String>,
        credentials: Credentials,
        verifier: Arc<dyn HostKeyVerifier>,
    ) -> Self {
        Self {
            address: address.into(),
            credentials,
            verifier,
        }
    }
    async fn establish(&self, size: TerminalSize) -> Result<TerminalSession, ConnectError> {
        let handle = self.authenticated_handle().await?;
        let channel = match open_terminal(&handle, size).await {
            Ok(channel) => channel,
            Err(error) => {
                disconnect(&handle).await;
                return Err(error);
            }
        };
        let (command_sender, commands) = mpsc::channel(QUEUE_LENGTH);
        let (events_sender, events) = mpsc::channel(QUEUE_LENGTH);
        tokio::spawn(pump(handle, channel, commands, events_sender));
        Ok(TerminalSession::from_channels(command_sender, events))
    }
    async fn authenticated_handle(&self) -> Result<Handle<TrustHandler>, ConnectError> {
        let address = lookup_host(&self.address)
            .await
            .map_err(|error| ConnectError::Network(error.to_string()))?
            .next()
            .ok_or_else(|| ConnectError::Network("address not found".into()))?;
        let config = Arc::new(client::Config {
            keepalive_interval: Some(KEEPALIVE_INTERVAL),
            keepalive_max: KEEPALIVE_MAX_MISSED,
            nodelay: true,
            ..client::Config::default()
        });
        let handler = TrustHandler {
            verifier: self.verifier.clone(),
        };
        let mut handle = match timeout(CONNECT_TIMEOUT, client::connect(config, address, handler))
            .await
        {
            Err(_) => return Err(ConnectError::Network("timed out".into())),
            Ok(Ok(handle)) => handle,
            Ok(Err(HandlerError::HostKeyRejected)) => return Err(ConnectError::HostKeyRejected),
            Ok(Err(HandlerError::Ssh(error))) => {
                return Err(ConnectError::Network(error.to_string()));
            }
        };
        // `russh` keeps its own copy of the password for the length of the exchange.
        let outcome = handle
            .authenticate_password(self.credentials.user(), self.credentials.password())
            .await;
        match outcome {
            Ok(result) if result.success() => Ok(handle),
            Ok(_) => {
                disconnect(&handle).await;
                Err(ConnectError::AuthenticationFailed)
            }
            Err(error) => Err(ConnectError::Network(error.to_string())),
        }
    }
}
impl Connector for SshConnector {
    fn connect(
        &mut self,
        size: TerminalSize,
    ) -> impl Future<Output = Result<TerminalSession, ConnectError>> + Send {
        self.establish(size)
    }
}

#[derive(Debug)]
enum HandlerError {
    HostKeyRejected,
    Ssh(russh::Error),
}
impl From<russh::Error> for HandlerError {
    fn from(error: russh::Error) -> Self {
        Self::Ssh(error)
    }
}

struct TrustHandler {
    verifier: Arc<dyn HostKeyVerifier>,
}
impl client::Handler for TrustHandler {
    type Error = HandlerError;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // Host certificates are not supported; refusing them is the safe answer.
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return Err(HandlerError::HostKeyRejected);
        };
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        if self.verifier.verify(&fingerprint) {
            Ok(true)
        } else {
            Err(HandlerError::HostKeyRejected)
        }
    }
}

async fn open_terminal(
    handle: &Handle<TrustHandler>,
    size: TerminalSize,
) -> Result<russh::Channel<client::Msg>, ConnectError> {
    let refused = |error: russh::Error| ConnectError::Refused(error.to_string());
    let channel = handle.channel_open_session().await.map_err(refused)?;
    channel
        .request_pty(
            true,
            TERMINAL_TYPE,
            u32::from(size.cols()),
            u32::from(size.rows()),
            0,
            0,
            &[],
        )
        .await
        .map_err(refused)?;
    channel.exec(true, REMOTE_COMMAND).await.map_err(refused)?;
    Ok(channel)
}

async fn disconnect(handle: &Handle<TrustHandler>) {
    let _ = handle.disconnect(Disconnect::ByApplication, "", "en").await;
}

/// Forwards input to the channel and output to the event queue until either side ends. Releases
/// the connection on every exit path.
async fn pump(
    handle: Handle<TrustHandler>,
    mut channel: russh::Channel<client::Msg>,
    mut commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<TerminalEvent>,
) {
    let mut exit_status = None;
    let reason = loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Input(bytes)) => {
                    if channel.data_bytes(bytes).await.is_err() {
                        break EndReason::Lost;
                    }
                }
                Some(Command::Resize(size)) => {
                    let resized = channel
                        .window_change(u32::from(size.cols()), u32::from(size.rows()), 0, 0)
                        .await;
                    if resized.is_err() {
                        break EndReason::Lost;
                    }
                }
                Some(Command::Close) | None => break EndReason::Closed,
            },
            message = channel.wait() => match message {
                Some(ChannelMsg::Data { data }) => {
                    if events.send(TerminalEvent::Output(data.to_vec())).await.is_err() {
                        break EndReason::Closed;
                    }
                }
                Some(ChannelMsg::ExitStatus { exit_status: code }) => exit_status = Some(code),
                Some(ChannelMsg::Close) | None => {
                    break exit_status.map_or(EndReason::Lost, EndReason::Exited);
                }
                Some(_) => {}
            },
        }
    };
    disconnect(&handle).await;
    let _ = events.send(TerminalEvent::Ended(reason)).await;
}
