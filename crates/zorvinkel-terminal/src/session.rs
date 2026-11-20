use crate::{Connector, Credentials, TerminalSize, credentials::Secret, deposit::SessionKey};
use russh::{
    ChannelMsg, Disconnect,
    client::{self, Handle},
    keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
};
use std::{fmt, future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::lookup_host,
    sync::mpsc,
    time::timeout,
};

/// Attaches to the session `zorvinkel`, creating it when missing (ADR-030).
/// Mouse mode lets the wheel scroll through the history of `tmux`, which owns the screen.
pub(crate) const REMOTE_COMMAND: &str = r"tmux new-session -A -s zorvinkel \; set-option mouse on";
const TERMINAL_TYPE: &str = "xterm-256color";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(8);
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

/// A byte stream the SSH connection can run over: a TCP socket, or the Bluetooth tunnel.
pub trait ByteStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ByteStream for T {}

/// Opens a new transport to the device for each connection attempt (ADR-031): reconnecting
/// after a loss needs a fresh tunnel, because a tunnel does not move between transports.
/// The future a [`Dialer`] returns.
pub type Dialing<'a> =
    Pin<Box<dyn Future<Output = Result<Box<dyn ByteStream>, ConnectError>> + Send + 'a>>;

pub trait Dialer: Send + Sync {
    fn dial(&self) -> Dialing<'_>;
}

/// Connects to the device's `sshd` (ADR-030). A password is used once to deposit an ephemeral
/// session key (ADR-032); reconnects and the final cleanup use that key.
pub struct SshConnector {
    address: String,
    /// Replaces the TCP connection to `address` when set.
    dialer: Option<Arc<dyn Dialer>>,
    credentials: Credentials,
    verifier: Arc<dyn HostKeyVerifier>,
    /// Set once a session key has been deposited on the device and not yet removed.
    deposited: Option<SessionKey>,
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
            dialer: None,
            credentials,
            verifier,
            deposited: None,
        }
    }
    /// Connects over streams the `dialer` opens instead of a TCP connection.
    pub fn with_dialer(
        dialer: Arc<dyn Dialer>,
        credentials: Credentials,
        verifier: Arc<dyn HostKeyVerifier>,
    ) -> Self {
        Self {
            address: String::new(),
            dialer: Some(dialer),
            credentials,
            verifier,
            deposited: None,
        }
    }
    async fn establish(&mut self, size: TerminalSize) -> Result<TerminalSession, ConnectError> {
        let handle = self.authenticated_handle().await?;
        if self.credentials.is_password()
            && let Err(error) = self.replace_password_by_session_key(&handle).await
        {
            disconnect(&handle).await;
            return Err(error);
        }
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
    /// Afterwards the password is dropped; if depositing fails it stays so the caller can decide.
    async fn replace_password_by_session_key(
        &mut self,
        handle: &Handle<TrustHandler>,
    ) -> Result<(), ConnectError> {
        let refused = || ConnectError::Refused("could not deposit a session key".into());
        let key = SessionKey::generate().ok_or_else(refused)?;
        match run_command(handle, key.deposit_command()).await? {
            0 => {}
            _ => return Err(refused()),
        }
        self.credentials = self.credentials.session_key(key.private.clone());
        self.deposited = Some(key);
        Ok(())
    }
    async fn remove_session_key(&mut self) {
        let Some(key) = self.deposited.take() else {
            return;
        };
        let cleanup = async {
            let handle = self.authenticated_handle().await.ok()?;
            let removed = run_command(&handle, &key.removal_command()).await;
            disconnect(&handle).await;
            removed.ok()
        };
        // Best effort: a leftover public key is harmless, its private half is gone with us.
        let _ = timeout(CLEANUP_TIMEOUT, cleanup).await;
    }
    async fn authenticated_handle(&self) -> Result<Handle<TrustHandler>, ConnectError> {
        let config = Arc::new(client::Config {
            keepalive_interval: Some(KEEPALIVE_INTERVAL),
            keepalive_max: KEEPALIVE_MAX_MISSED,
            nodelay: true,
            ..client::Config::default()
        });
        let handler = TrustHandler {
            verifier: self.verifier.clone(),
        };
        let connecting = self.start_connection(config, handler);
        let mut handle = match timeout(CONNECT_TIMEOUT, connecting).await {
            Err(_) => return Err(ConnectError::Network("timed out".into())),
            Ok(Ok(handle)) => handle,
            Ok(Err(HandlerErrorOr::Connect(error))) => return Err(error),
            Ok(Err(HandlerErrorOr::Handler(HandlerError::HostKeyRejected))) => {
                return Err(ConnectError::HostKeyRejected);
            }
            Ok(Err(HandlerErrorOr::Handler(HandlerError::Ssh(error)))) => {
                return Err(ConnectError::Network(error.to_string()));
            }
        };
        let outcome = self.authenticate(&mut handle).await;
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
impl SshConnector {
    /// The SSH handshake over a TCP connection or a stream from the dialer.
    async fn start_connection(
        &self,
        config: Arc<client::Config>,
        handler: TrustHandler,
    ) -> Result<Handle<TrustHandler>, HandlerErrorOr> {
        if let Some(dialer) = &self.dialer {
            let stream = dialer.dial().await.map_err(HandlerErrorOr::Connect)?;
            return client::connect_stream(config, stream, handler)
                .await
                .map_err(HandlerErrorOr::Handler);
        }
        let address = lookup_host(&self.address)
            .await
            .map_err(|error| HandlerErrorOr::Connect(ConnectError::Network(error.to_string())))?
            .next()
            .ok_or_else(|| {
                HandlerErrorOr::Connect(ConnectError::Network("address not found".into()))
            })?;
        client::connect(config, address, handler)
            .await
            .map_err(HandlerErrorOr::Handler)
    }
}
enum HandlerErrorOr {
    Connect(ConnectError),
    Handler(HandlerError),
}
impl SshConnector {
    async fn authenticate(
        &self,
        handle: &mut Handle<TrustHandler>,
    ) -> Result<client::AuthResult, russh::Error> {
        let user = self.credentials.user();
        match self.credentials.secret() {
            // `russh` keeps its own copy of the password for the length of the exchange.
            Secret::Password(password) => {
                handle.authenticate_password(user, password.as_str()).await
            }
            Secret::Key(key) => {
                let hash = handle.best_supported_rsa_hash().await?.flatten();
                handle
                    .authenticate_publickey(user, PrivateKeyWithHashAlg::new(key.clone(), hash))
                    .await
            }
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
    fn release(&mut self) -> impl Future<Output = ()> + Send {
        self.remove_session_key()
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

/// Runs a command without a terminal and returns its exit status.
async fn run_command(handle: &Handle<TrustHandler>, command: &str) -> Result<u32, ConnectError> {
    let refused = |error: russh::Error| ConnectError::Refused(error.to_string());
    let exchange = async {
        let mut channel = handle.channel_open_session().await.map_err(refused)?;
        channel.exec(true, command).await.map_err(refused)?;
        let mut status = None;
        while let Some(message) = channel.wait().await {
            match message {
                ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        status.ok_or_else(|| ConnectError::Refused("no exit status".into()))
    };
    timeout(COMMAND_TIMEOUT, exchange)
        .await
        .map_err(|_| ConnectError::Network("command timed out".into()))?
}
