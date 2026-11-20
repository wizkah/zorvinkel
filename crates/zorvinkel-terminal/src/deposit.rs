//! Session key (ADR-032, ADR-033): the password is used once; reconnects use a key that lives in
//! client memory. Its public half is removed again when the terminal ends, unless easy login
//! keeps it on the device.
use russh::keys::{Algorithm, PrivateKey, ssh_key::LineEnding};
use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroizing;

/// Shared between the connector and the caller: whether the session key stays on the device
/// after the terminal ends ("easy login"), and the key itself so the caller can store it.
#[derive(Clone, Default)]
pub struct KeyControl(Arc<Mutex<KeyState>>);
#[derive(Default)]
struct KeyState {
    keep: bool,
    private: Option<PrivateKey>,
}
impl KeyControl {
    /// `true`: the entry on the device stays when the terminal ends. `false`: it is removed.
    pub fn keep(&self, keep: bool) {
        if let Ok(mut state) = self.0.lock() {
            state.keep = keep;
        }
    }
    pub fn is_kept(&self) -> bool {
        self.0.lock().is_ok_and(|state| state.keep)
    }
    /// The private key that carries the terminal, in OpenSSH format; `None` until the key
    /// exists (after the first login).
    pub fn private_key(&self) -> Option<Zeroizing<String>> {
        let state = self.0.lock().ok()?;
        state.private.as_ref()?.to_openssh(LineEnding::LF).ok()
    }
    pub(crate) fn set_key(&self, key: PrivateKey) {
        if let Ok(mut state) = self.0.lock() {
            state.private = Some(key);
        }
    }
}

pub(crate) struct SessionKey {
    pub(crate) private: PrivateKey,
    /// Base64 blob of the public key; identifies the entry when it is removed.
    blob: String,
    deposit_command: String,
}
impl SessionKey {
    pub(crate) fn generate() -> Option<Self> {
        Self::from_private(PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).ok()?)
    }
    /// Wraps a key that is already on the device (easy login), so it can be removed again.
    pub(crate) fn from_private(private: PrivateKey) -> Option<Self> {
        let line = private.public_key().to_openssh().ok()?;
        let blob = line.split_whitespace().nth(1)?.to_owned();
        // The line comes from our own encoder; the check keeps shell quoting trivially safe.
        let safe = |text: &str| {
            text.chars()
                .all(|c| c.is_ascii_alphanumeric() || "+/=- @.".contains(c))
        };
        if !safe(&line) {
            return None;
        }
        let seconds = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        let entry = format!("restrict,pty {line} zorvinkel-session-{seconds}");
        // The leading newline protects a file whose last line lacks one.
        let deposit_command = format!(
            "umask 077 && mkdir -p ~/.ssh && touch ~/.ssh/authorized_keys \
             && printf '\n%s\n' '{entry}' >> ~/.ssh/authorized_keys"
        );
        Some(Self {
            private,
            blob,
            deposit_command,
        })
    }
    pub(crate) fn deposit_command(&self) -> &str {
        &self.deposit_command
    }
    pub(crate) fn removal_command(&self) -> String {
        format!(r"sed -i '\#{}#d' ~/.ssh/authorized_keys", self.blob)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_contain_only_the_public_key() {
        let key = SessionKey::generate().unwrap();
        let deposit = key.deposit_command();
        assert!(deposit.contains("restrict,pty ssh-ed25519 "));
        assert!(deposit.contains(&key.blob));
        let private = key
            .private
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .unwrap();
        assert!(!deposit.contains(private.lines().nth(1).unwrap()));
    }
    #[test]
    fn removal_targets_exactly_this_key() {
        let key = SessionKey::generate().unwrap();
        let removal = key.removal_command();
        assert!(removal.starts_with(r"sed -i '\#"));
        assert!(removal.contains(&key.blob));
        assert_ne!(
            removal,
            SessionKey::generate().unwrap().removal_command(),
            "every terminal gets its own key"
        );
    }
}
