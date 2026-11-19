use crate::{
    TerminalSize,
    session::{Command, ConnectError, EndReason, SessionClosed, TerminalEvent, TerminalSession},
};
use std::{future::Future, pin::pin, time::Duration};
use tokio::{sync::mpsc, time::sleep};

const QUEUE_LENGTH: usize = 64;

/// Opens a new terminal session; used again after every interruption.
pub trait Connector: Send + 'static {
    fn connect(
        &mut self,
        size: TerminalSize,
    ) -> impl Future<Output = Result<TerminalSession, ConnectError>> + Send;
    /// Called once when the terminal ends for good, to clean up on the device (ADR-032).
    fn release(&mut self) -> impl Future<Output = ()> + Send {
        async {}
    }
}

/// Waiting time before reconnection attempts: doubles from `first` up to `cap`, gives up after
/// `max_attempts`.
#[derive(Clone, Copy, Debug)]
pub struct RetryPolicy {
    pub first: Duration,
    pub cap: Duration,
    pub max_attempts: u32,
}
impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            first: Duration::from_secs(1),
            cap: Duration::from_secs(15),
            max_attempts: 8,
        }
    }
}
impl RetryPolicy {
    /// Waiting time before the attempt following failed attempt number `attempt` (counted from
    /// one), or `None` when the user has to decide.
    pub fn delay(&self, attempt: u32) -> Option<Duration> {
        if attempt == 0 || attempt > self.max_attempts {
            return None;
        }
        let doublings = (attempt - 1).min(16);
        Some(self.first.saturating_mul(1 << doublings).min(self.cap))
    }
}

#[derive(Debug)]
pub enum ConnectionStatus {
    Connecting,
    /// A session is attached. After a reconnect `tmux` redraws the screen, so the terminal
    /// emulation should start from an empty screen on every `Connected`.
    Connected,
    Reconnecting {
        attempt: u32,
    },
    /// The remote program ended with this exit status; no reconnect follows.
    Finished(u32),
    /// Connecting failed for good; no reconnect follows.
    Failed(ConnectError),
}

#[derive(Debug)]
pub enum ResumableEvent {
    Status(ConnectionStatus),
    Output(Vec<u8>),
}

/// A terminal that reconnects after interruptions and reattaches to the same `tmux` session
/// (ADR-030). Keystrokes typed while no session is attached are dropped. Dropping it ends the
/// connection.
pub struct ResumableTerminal {
    commands: mpsc::Sender<Command>,
    events: mpsc::Receiver<ResumableEvent>,
}
impl ResumableTerminal {
    /// Starts connecting in the background; needs a running Tokio runtime.
    pub fn start<C: Connector>(connector: C, size: TerminalSize, policy: RetryPolicy) -> Self {
        let (commands, command_queue) = mpsc::channel(QUEUE_LENGTH);
        let (event_sender, events) = mpsc::channel(QUEUE_LENGTH);
        tokio::spawn(supervise(
            connector,
            size,
            policy,
            command_queue,
            event_sender,
        ));
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
    /// `None` once the terminal stopped for good.
    pub async fn next_event(&mut self) -> Option<ResumableEvent> {
        self.events.recv().await
    }
    async fn send(&self, command: Command) -> Result<(), SessionClosed> {
        self.commands.send(command).await.map_err(|_| SessionClosed)
    }
}

enum Outcome {
    Finished(u32),
    Lost,
    Stopped,
}

async fn supervise<C: Connector>(
    mut connector: C,
    size: TerminalSize,
    policy: RetryPolicy,
    commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<ResumableEvent>,
) {
    run(&mut connector, size, policy, commands, events).await;
    connector.release().await;
}

async fn run<C: Connector>(
    connector: &mut C,
    mut size: TerminalSize,
    policy: RetryPolicy,
    mut commands: mpsc::Receiver<Command>,
    events: mpsc::Sender<ResumableEvent>,
) {
    // `attempt` numbers the tries shown to the user; `failures` counts consecutive failed tries
    // and drives the waiting time.
    let mut attempt = 0;
    let mut failures = 0;
    loop {
        let status = match attempt {
            0 => ConnectionStatus::Connecting,
            _ => ConnectionStatus::Reconnecting { attempt },
        };
        if !emit(&events, ResumableEvent::Status(status)).await {
            return;
        }
        let connected = interruptible(connector.connect(size), &mut commands, &mut size).await;
        let result = match connected {
            Some(result) => result,
            None => return,
        };
        match result {
            Ok(session) => {
                failures = 0;
                if !emit(&events, ResumableEvent::Status(ConnectionStatus::Connected)).await {
                    return;
                }
                match attached(session, &mut commands, &events, &mut size).await {
                    Outcome::Finished(code) => {
                        emit(
                            &events,
                            ResumableEvent::Status(ConnectionStatus::Finished(code)),
                        )
                        .await;
                        return;
                    }
                    Outcome::Stopped => return,
                    // Reconnect at once; only repeated failures back off.
                    Outcome::Lost => attempt = 1,
                }
            }
            Err(error) if error.is_retryable() => {
                failures += 1;
                attempt += 1;
                let Some(delay) = policy.delay(failures) else {
                    emit(
                        &events,
                        ResumableEvent::Status(ConnectionStatus::Failed(error)),
                    )
                    .await;
                    return;
                };
                if interruptible(sleep(delay), &mut commands, &mut size)
                    .await
                    .is_none()
                {
                    return;
                }
            }
            Err(error) => {
                emit(
                    &events,
                    ResumableEvent::Status(ConnectionStatus::Failed(error)),
                )
                .await;
                return;
            }
        }
    }
}

/// Relays between the attached session and the caller until the session ends or the caller stops.
async fn attached(
    mut session: TerminalSession,
    commands: &mut mpsc::Receiver<Command>,
    events: &mpsc::Sender<ResumableEvent>,
    size: &mut TerminalSize,
) -> Outcome {
    loop {
        tokio::select! {
            event = session.next_event() => match event {
                Some(TerminalEvent::Output(bytes)) => {
                    if !emit(events, ResumableEvent::Output(bytes)).await {
                        return Outcome::Stopped;
                    }
                }
                Some(TerminalEvent::Ended(EndReason::Exited(code))) => {
                    return Outcome::Finished(code);
                }
                Some(TerminalEvent::Ended(_)) | None => return Outcome::Lost,
            },
            command = commands.recv() => match command {
                Some(Command::Input(bytes)) => {
                    let _ = session.input(bytes).await;
                }
                Some(Command::Resize(new_size)) => {
                    *size = new_size;
                    let _ = session.resize(new_size).await;
                }
                Some(Command::Close) | None => {
                    session.close().await;
                    return Outcome::Stopped;
                }
            },
        }
    }
}

/// Runs `future` while no session is attached: input is dropped, resizes are remembered. `None`
/// when the caller asked to stop.
async fn interruptible<F: Future>(
    future: F,
    commands: &mut mpsc::Receiver<Command>,
    size: &mut TerminalSize,
) -> Option<F::Output> {
    let mut future = pin!(future);
    loop {
        tokio::select! {
            output = &mut future => return Some(output),
            command = commands.recv() => match command {
                Some(Command::Input(_)) => {}
                Some(Command::Resize(new_size)) => *size = new_size,
                Some(Command::Close) | None => return None,
            },
        }
    }
}

/// `false` when nobody listens any more.
async fn emit(events: &mpsc::Sender<ResumableEvent>, event: ResumableEvent) -> bool {
    events.send(event).await.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn delays_double_up_to_the_cap_and_end() {
        let policy = RetryPolicy {
            first: Duration::from_secs(1),
            cap: Duration::from_secs(5),
            max_attempts: 5,
        };
        let seconds: Vec<_> = (0..=6)
            .map(|attempt| policy.delay(attempt).map(|delay| delay.as_secs()))
            .collect();
        assert_eq!(
            seconds,
            [None, Some(1), Some(2), Some(4), Some(5), Some(5), None]
        );
    }

    /// The device side of a fake session.
    struct Remote {
        input: mpsc::Receiver<Command>,
        output: mpsc::Sender<TerminalEvent>,
    }
    fn fake_session() -> (TerminalSession, Remote) {
        let (command_sender, input) = mpsc::channel(8);
        let (output, events) = mpsc::channel(8);
        (
            TerminalSession::from_channels(command_sender, events),
            Remote { input, output },
        )
    }

    struct Scripted(VecDeque<Result<TerminalSession, ConnectError>>);
    impl Connector for Scripted {
        async fn connect(&mut self, _size: TerminalSize) -> Result<TerminalSession, ConnectError> {
            self.0
                .pop_front()
                .unwrap_or(Err(ConnectError::Network("no more attempts".into())))
        }
    }

    fn size() -> TerminalSize {
        TerminalSize::new(80, 24).unwrap()
    }
    fn start(results: Vec<Result<TerminalSession, ConnectError>>) -> ResumableTerminal {
        ResumableTerminal::start(Scripted(results.into()), size(), RetryPolicy::default())
    }
    async fn status(terminal: &mut ResumableTerminal) -> ConnectionStatus {
        match terminal.next_event().await {
            Some(ResumableEvent::Status(status)) => status,
            other => panic!("expected a status, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn relays_output_and_input() {
        let (session, mut remote) = fake_session();
        let mut terminal = start(vec![Ok(session)]);
        assert!(matches!(
            status(&mut terminal).await,
            ConnectionStatus::Connecting
        ));
        assert!(matches!(
            status(&mut terminal).await,
            ConnectionStatus::Connected
        ));

        remote
            .output
            .send(TerminalEvent::Output(b"$ ".to_vec()))
            .await
            .unwrap();
        assert!(matches!(
            terminal.next_event().await,
            Some(ResumableEvent::Output(bytes)) if bytes == b"$ "
        ));
        terminal.input(b"ls\r".to_vec()).await.unwrap();
        assert!(matches!(
            remote.input.recv().await,
            Some(Command::Input(bytes)) if bytes == b"ls\r"
        ));
    }

    #[tokio::test]
    async fn reconnects_after_a_lost_connection() {
        let (first, remote) = fake_session();
        let (second, _second_remote) = fake_session();
        let mut terminal = start(vec![Ok(first), Ok(second)]);
        status(&mut terminal).await;
        status(&mut terminal).await;

        remote
            .output
            .send(TerminalEvent::Ended(EndReason::Lost))
            .await
            .unwrap();
        assert!(matches!(
            status(&mut terminal).await,
            ConnectionStatus::Reconnecting { attempt: 1 }
        ));
        assert!(matches!(
            status(&mut terminal).await,
            ConnectionStatus::Connected
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn retries_with_delays_and_then_gives_up() {
        let policy = RetryPolicy {
            first: Duration::from_secs(1),
            cap: Duration::from_secs(2),
            max_attempts: 2,
        };
        let mut terminal = ResumableTerminal::start(Scripted(VecDeque::new()), size(), policy);
        assert!(matches!(
            status(&mut terminal).await,
            ConnectionStatus::Connecting
        ));
        assert!(matches!(
            status(&mut terminal).await,
            ConnectionStatus::Reconnecting { attempt: 1 }
        ));
        assert!(matches!(
            status(&mut terminal).await,
            ConnectionStatus::Reconnecting { attempt: 2 }
        ));
        assert!(matches!(
            status(&mut terminal).await,
            ConnectionStatus::Failed(ConnectError::Network(_))
        ));
        assert!(terminal.next_event().await.is_none());
    }

    #[tokio::test]
    async fn wrong_password_is_not_retried() {
        let mut terminal = start(vec![Err(ConnectError::AuthenticationFailed)]);
        status(&mut terminal).await;
        assert!(matches!(
            status(&mut terminal).await,
            ConnectionStatus::Failed(ConnectError::AuthenticationFailed)
        ));
        assert!(terminal.next_event().await.is_none());
    }

    #[tokio::test]
    async fn a_finished_program_does_not_reconnect() {
        let (session, remote) = fake_session();
        let mut terminal = start(vec![Ok(session)]);
        status(&mut terminal).await;
        status(&mut terminal).await;

        remote
            .output
            .send(TerminalEvent::Ended(EndReason::Exited(0)))
            .await
            .unwrap();
        assert!(matches!(
            status(&mut terminal).await,
            ConnectionStatus::Finished(0)
        ));
        assert!(terminal.next_event().await.is_none());
    }

    #[tokio::test]
    async fn closing_releases_the_session() {
        let (session, mut remote) = fake_session();
        let mut terminal = start(vec![Ok(session)]);
        status(&mut terminal).await;
        status(&mut terminal).await;

        terminal.close().await;
        assert!(matches!(remote.input.recv().await, Some(Command::Close)));
        assert!(terminal.next_event().await.is_none());
    }

    #[tokio::test]
    async fn closing_while_connecting_stops_at_once() {
        struct Never;
        impl Connector for Never {
            async fn connect(&mut self, _: TerminalSize) -> Result<TerminalSession, ConnectError> {
                std::future::pending().await
            }
        }
        let mut terminal = ResumableTerminal::start(Never, size(), RetryPolicy::default());
        status(&mut terminal).await;
        terminal.close().await;
        assert!(terminal.next_event().await.is_none());
    }
}
