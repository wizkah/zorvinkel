//! Interactive terminal over SSH (ADR-030).
//!
//! The client runs SSH itself: it connects to the device's `sshd`, authenticates with the
//! operating-system account and attaches to a `tmux` session, so a reconnect after an
//! interruption reaches the same programs. This crate holds no GUI code; the terminal widget
//! reads the screen state from [`Screen`].
mod credentials;
mod resume;
mod screen;
mod session;

pub use credentials::{Credentials, InvalidCredentials};
pub use resume::{ConnectionStatus, Connector, ResumableEvent, ResumableTerminal, RetryPolicy};
pub use screen::{Screen, TerminalSize};
pub use session::{
    ConnectError, EndReason, HostKeyVerifier, SessionClosed, SshConnector, TerminalEvent,
    TerminalSession,
};
