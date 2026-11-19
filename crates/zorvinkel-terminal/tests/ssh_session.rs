//! The real SSH path against an in-process server that stands in for the device's `sshd`.
use russh::{
    Channel, ChannelId,
    keys::{Algorithm, PrivateKey},
    server::{self, Auth, Msg, Server as _, Session},
};
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;
use zorvinkel_terminal::{
    ConnectError, ConnectionStatus, Credentials, EndReason, ResumableEvent, ResumableTerminal,
    RetryPolicy, SshConnector, TerminalEvent, TerminalSize,
};

const PASSWORD: &str = "correct horse";

/// Records what the client asked for and answers like a tiny shell: echoes input, `q` quits.
#[derive(Clone, Default)]
struct FakeDevice {
    seen: Arc<Mutex<Seen>>,
}
#[derive(Default)]
struct Seen {
    pty: Option<(String, u32, u32)>,
    command: Option<String>,
    window: Option<(u32, u32)>,
    /// Login methods in the order they succeeded.
    logins: Vec<&'static str>,
    /// Commands run without a terminal (session key deposit and removal).
    maintenance: Vec<String>,
}
impl server::Server for FakeDevice {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}
impl server::Handler for FakeDevice {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if password == PASSWORD {
            self.seen.lock().unwrap().logins.push("password");
            Auth::Accept
        } else {
            Auth::reject()
        })
    }
    async fn auth_publickey(
        &mut self,
        _user: &str,
        _key: server::VerifiedPublicKey<'_>,
    ) -> Result<Auth, Self::Error> {
        self.seen.lock().unwrap().logins.push("key");
        Ok(Auth::Accept)
    }
    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }
    async fn pty_request(
        &mut self,
        channel: ChannelId,
        term: &str,
        cols: u32,
        rows: u32,
        _: u32,
        _: u32,
        _: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.seen.lock().unwrap().pty = Some((term.to_owned(), cols, rows));
        session.channel_success(channel)
    }
    async fn exec_request(
        &mut self,
        channel: ChannelId,
        command: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(command).into_owned();
        session.channel_success(channel)?;
        if command.starts_with("umask") || command.starts_with("sed") {
            self.seen.lock().unwrap().maintenance.push(command);
            session.exit_status_request(channel, 0)?;
            return session.close(channel);
        }
        self.seen.lock().unwrap().command = Some(command);
        session.data(channel, &b"ready\r\n"[..])
    }
    async fn window_change_request(
        &mut self,
        _channel: ChannelId,
        cols: u32,
        rows: u32,
        _: u32,
        _: u32,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.seen.lock().unwrap().window = Some((cols, rows));
        Ok(())
    }
    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if data == b"q" {
            session.exit_status_request(channel, 7)?;
            session.close(channel)
        } else {
            session.data(channel, data.to_vec())
        }
    }
}

async fn start_device() -> (String, FakeDevice, String) {
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let fingerprint = key
        .public_key()
        .fingerprint(russh::keys::HashAlg::Sha256)
        .to_string();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: std::time::Duration::ZERO,
        auth_rejection_time_initial: Some(std::time::Duration::ZERO),
        ..Default::default()
    });
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let device = FakeDevice::default();
    let mut running = device.clone();
    tokio::spawn(async move { running.run_on_socket(config, &listener).await });
    (address, device, fingerprint)
}

fn size() -> TerminalSize {
    TerminalSize::new(100, 30).unwrap()
}
fn connector(address: &str, password: &str, trusted: &str) -> SshConnector {
    let trusted = trusted.to_owned();
    SshConnector::new(
        address,
        Credentials::password("pi", password.to_owned()).unwrap(),
        Arc::new(move |fingerprint: &str| fingerprint == trusted),
    )
}
async fn status(terminal: &mut ResumableTerminal) -> ConnectionStatus {
    loop {
        if let Some(ResumableEvent::Status(status)) = terminal.next_event().await {
            return status;
        }
    }
}
async fn expect_output(terminal: &mut ResumableTerminal, expected: &[u8]) {
    let mut received = Vec::new();
    while !received.ends_with(expected) {
        match terminal.next_event().await {
            Some(ResumableEvent::Output(bytes)) => received.extend(bytes),
            other => panic!("expected output, got {other:?} after {received:?}"),
        }
    }
}

#[tokio::test]
async fn attaches_to_tmux_and_relays_a_session() {
    let (address, device, fingerprint) = start_device().await;
    let mut terminal = ResumableTerminal::start(
        connector(&address, PASSWORD, &fingerprint),
        size(),
        RetryPolicy::default(),
    );
    assert!(matches!(
        status(&mut terminal).await,
        ConnectionStatus::Connecting
    ));
    assert!(matches!(
        status(&mut terminal).await,
        ConnectionStatus::Connected
    ));
    expect_output(&mut terminal, b"ready\r\n").await;

    terminal.input(b"ls".to_vec()).await.unwrap();
    expect_output(&mut terminal, b"ls").await;
    terminal
        .resize(TerminalSize::new(120, 40).unwrap())
        .await
        .unwrap();
    terminal.input(b"q".to_vec()).await.unwrap();
    assert!(matches!(
        status(&mut terminal).await,
        ConnectionStatus::Finished(7)
    ));

    let seen = device.seen.lock().unwrap();
    assert_eq!(seen.pty, Some(("xterm-256color".to_owned(), 100, 30)));
    assert_eq!(
        seen.command.as_deref(),
        Some("tmux new-session -A -s zorvinkel")
    );
    assert_eq!(seen.window, Some((120, 40)));
}

#[tokio::test]
async fn wrong_password_fails_without_retrying() {
    let (address, _, fingerprint) = start_device().await;
    let mut terminal = ResumableTerminal::start(
        connector(&address, "wrong", &fingerprint),
        size(),
        RetryPolicy::default(),
    );
    status(&mut terminal).await;
    assert!(matches!(
        status(&mut terminal).await,
        ConnectionStatus::Failed(ConnectError::AuthenticationFailed)
    ));
}

#[tokio::test]
async fn untrusted_host_key_is_rejected_before_authentication() {
    let (address, _, _) = start_device().await;
    let mut terminal = ResumableTerminal::start(
        connector(&address, PASSWORD, "SHA256:someone-else"),
        size(),
        RetryPolicy::default(),
    );
    status(&mut terminal).await;
    assert!(matches!(
        status(&mut terminal).await,
        ConnectionStatus::Failed(ConnectError::HostKeyRejected)
    ));
}

#[tokio::test]
async fn closing_locally_ends_the_session() {
    use zorvinkel_terminal::Connector;
    let (address, _, fingerprint) = start_device().await;
    let mut connector = connector(&address, PASSWORD, &fingerprint);
    let mut session = connector.connect(size()).await.unwrap();
    session.close().await;
    loop {
        match session.next_event().await {
            Some(TerminalEvent::Ended(reason)) => {
                assert_eq!(reason, EndReason::Closed);
                break;
            }
            Some(TerminalEvent::Output(_)) => {}
            None => panic!("ended without a reason"),
        }
    }
}

#[tokio::test]
async fn logs_in_with_a_private_key() {
    use russh::keys::ssh_key::LineEnding;
    let (address, _, fingerprint) = start_device().await;
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    let text = key.to_openssh(LineEnding::LF).unwrap();
    let credentials = Credentials::private_key("pi", &text, None).unwrap();
    let mut terminal = ResumableTerminal::start(
        SshConnector::new(
            address,
            credentials,
            Arc::new(move |presented: &str| presented == fingerprint),
        ),
        size(),
        RetryPolicy::default(),
    );
    status(&mut terminal).await;
    assert!(matches!(
        status(&mut terminal).await,
        ConnectionStatus::Connected
    ));
}

/// Against a real device: `ZORVINKEL_TEST_SSH_ADDRESS` (host:port), `ZORVINKEL_TEST_SSH_USER`
/// and `ZORVINKEL_TEST_SSH_KEY_FILE` (unencrypted private key). Run with `--ignored --nocapture`.
/// Detaches from `tmux` at once; without `tmux` the remote command ends with status 127.
#[tokio::test]
#[ignore = "needs a real device"]
async fn real_device_session() {
    let variable = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} not set"));
    let key_text = std::fs::read_to_string(variable("ZORVINKEL_TEST_SSH_KEY_FILE")).unwrap();
    let credentials =
        Credentials::private_key(&variable("ZORVINKEL_TEST_SSH_USER"), &key_text, None).unwrap();
    let connector = SshConnector::new(
        variable("ZORVINKEL_TEST_SSH_ADDRESS"),
        credentials,
        Arc::new(|fingerprint: &str| {
            println!("host key {fingerprint}");
            true
        }),
    );
    let mut terminal = ResumableTerminal::start(connector, size(), RetryPolicy::default());
    status(&mut terminal).await;
    assert!(matches!(
        status(&mut terminal).await,
        ConnectionStatus::Connected
    ));
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    // Fails when the remote command already ended (no `tmux` on the device).
    let _ = terminal.input(b"\x02d".to_vec()).await;
    let mut screen = zorvinkel_terminal::Screen::new(size());
    loop {
        match terminal.next_event().await {
            Some(ResumableEvent::Output(bytes)) => screen.process(&bytes),
            Some(ResumableEvent::Status(ConnectionStatus::Finished(code))) => {
                println!(
                    "finished with status {code}\nscreen:\n{}",
                    screen.view().contents()
                );
                break;
            }
            other => panic!("unexpected {other:?}"),
        }
    }
}

#[tokio::test]
async fn password_is_replaced_by_a_session_key_and_the_key_is_removed_again() {
    use zorvinkel_terminal::Connector;
    let (address, device, fingerprint) = start_device().await;
    let mut connector = connector(&address, PASSWORD, &fingerprint);

    let first = connector.connect(size()).await.unwrap();
    first.close().await;
    let mut second = connector.connect(size()).await.unwrap();
    second.close().await;
    while second.next_event().await.is_some() {}
    connector.release().await;

    let seen = device.seen.lock().unwrap();
    assert_eq!(seen.logins[..2], ["password", "key"], "password only once");
    assert!(seen.logins[2..].iter().all(|login| *login == "key"));
    let [deposit, removal] = &seen.maintenance[..] else {
        panic!(
            "expected one deposit and one removal, got {:?}",
            seen.maintenance
        );
    };
    assert!(deposit.contains("restrict,pty ssh-ed25519 "));
    let blob = deposit.split_whitespace().rev().nth(3).unwrap();
    assert!(removal.contains(blob), "removes the deposited key");
}
