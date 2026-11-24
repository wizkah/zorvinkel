//! Registration with the operating-system account of the device (ADR-037): a client that the
//! device does not know yet proves it holds an administrator login and is registered, even
//! while registration is closed. The check of the password belongs to the operating-system
//! adapter; this module holds the rules around it.
use crate::{
    registration::authorize_by_local_admin,
    state::{ApplicationError, StateStore},
};
use std::collections::VecDeque;
use zorvinkel_core::registry::ClientKey;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginOutcome {
    /// The user and password are right and the account is an administrator.
    Accepted,
    /// The user or the password is not right.
    Rejected,
    /// The password is right, but the account is no administrator.
    NotAdministrator,
    /// Too many wrong attempts lately; the password was not even tried.
    Throttled,
    /// The device cannot check logins (no password login on `sshd`, not installed, timeout).
    Unavailable,
}

/// Checks the login of an operating-system account. Implementations never log the password and
/// keep it no longer than the check takes.
pub trait AdminLogin {
    fn verify(&mut self, user: &str, password: &str) -> LoginOutcome;
}

/// Registers `client` when the login is an administrator's. A client that was blocked before
/// stays blocked (the registry refuses it); the registry closes registration again.
pub fn register_with_login(
    store: &mut impl StateStore,
    login: &mut dyn AdminLogin,
    client: ClientKey,
    user: &str,
    password: &str,
) -> Result<LoginOutcome, ApplicationError> {
    let outcome = login.verify(user, password);
    if outcome == LoginOutcome::Accepted {
        authorize_by_local_admin(store, client)?;
    }
    Ok(outcome)
}

/// Wrong logins allowed per window before the device stops trying, device wide: someone in radio
/// range must not be able to guess a password by repetition. Registered clients do not use this
/// path, so the limit never blocks the regular use of the device (NFR-SEC-005).
pub const MAX_FAILURES: usize = 5;
pub const WINDOW_SECONDS: u64 = 300;

/// Counts wrong logins in a sliding window. Time is supplied by the caller.
#[derive(Debug, Default)]
pub struct FailureLimiter {
    failures: VecDeque<u64>,
}
impl FailureLimiter {
    /// `false` while the window holds the maximum number of failures.
    pub fn allows(&mut self, now: u64) -> bool {
        while self
            .failures
            .front()
            .is_some_and(|at| now.saturating_sub(*at) >= WINDOW_SECONDS)
        {
            self.failures.pop_front();
        }
        self.failures.len() < MAX_FAILURES
    }
    pub fn record_failure(&mut self, now: u64) {
        self.failures.push_back(now);
        if self.failures.len() > MAX_FAILURES {
            self.failures.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_limiter_stops_after_the_maximum_and_recovers_with_the_window() {
        let mut limiter = FailureLimiter::default();
        for second in 0..MAX_FAILURES as u64 {
            assert!(limiter.allows(second));
            limiter.record_failure(second);
        }
        assert!(!limiter.allows(10), "window is full");
        assert!(!limiter.allows(WINDOW_SECONDS - 1));
        // The oldest failure leaves the window first.
        assert!(limiter.allows(WINDOW_SECONDS));
    }
}
