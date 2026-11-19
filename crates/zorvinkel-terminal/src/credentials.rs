use russh::keys::{PrivateKey, decode_secret_key};
use std::{fmt, sync::Arc};
use zeroize::Zeroizing;

const MAX_USER_LENGTH: usize = 64;
const MAX_PASSWORD_LENGTH: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidCredentials {
    User,
    Password,
    /// Not an SSH private key, or the passphrase does not fit.
    Key,
}

pub(crate) enum Secret {
    Password(Zeroizing<String>),
    Key(Arc<PrivateKey>),
}

/// Operating-system account of the device. Passwords and keys stay in memory for the duration
/// of the terminal only (never stored) and are cleared when dropped.
#[derive(Clone)]
pub struct Credentials {
    user: String,
    secret: Arc<Secret>,
}
impl Credentials {
    pub fn password(user: &str, password: String) -> Result<Self, InvalidCredentials> {
        let user = valid_user(user)?;
        if password.len() > MAX_PASSWORD_LENGTH {
            return Err(InvalidCredentials::Password);
        }
        Ok(Self::from_secret(
            user,
            Secret::Password(Zeroizing::new(password)),
        ))
    }
    /// `key_text` is an OpenSSH or PEM private key; `passphrase` unlocks an encrypted one.
    pub fn private_key(
        user: &str,
        key_text: &str,
        passphrase: Option<&str>,
    ) -> Result<Self, InvalidCredentials> {
        let user = valid_user(user)?;
        let key = decode_secret_key(key_text, passphrase).map_err(|_| InvalidCredentials::Key)?;
        Ok(Self::from_secret(user, Secret::Key(Arc::new(key))))
    }
    pub fn user(&self) -> &str {
        &self.user
    }
    /// The ephemeral key that replaces a password after the first login (ADR-032).
    pub(crate) fn session_key(&self, key: PrivateKey) -> Self {
        Self::from_secret(self.user.clone(), Secret::Key(Arc::new(key)))
    }
    pub(crate) fn is_password(&self) -> bool {
        matches!(*self.secret, Secret::Password(_))
    }
    pub(crate) fn secret(&self) -> &Secret {
        &self.secret
    }
    fn from_secret(user: String, secret: Secret) -> Self {
        Self {
            user,
            secret: Arc::new(secret),
        }
    }
}
fn valid_user(user: &str) -> Result<String, InvalidCredentials> {
    let usable = !user.is_empty()
        && user.len() <= MAX_USER_LENGTH
        && !user.chars().any(|c| c.is_control() || c.is_whitespace());
    usable
        .then(|| user.to_owned())
        .ok_or(InvalidCredentials::User)
}
impl fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Credentials")
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_unusual_users() {
        for user in ["", "a b", "a\nb", &"x".repeat(65)] {
            assert_eq!(
                Credentials::password(user, "secret".into()).err(),
                Some(InvalidCredentials::User)
            );
        }
    }
    #[test]
    fn rejects_oversized_password() {
        assert_eq!(
            Credentials::password("pi", "x".repeat(1025)).err(),
            Some(InvalidCredentials::Password)
        );
    }
    #[test]
    fn rejects_text_that_is_no_key() {
        assert_eq!(
            Credentials::private_key("pi", "not a key", None).err(),
            Some(InvalidCredentials::Key)
        );
    }
    #[test]
    fn debug_output_hides_the_secret() {
        let credentials = Credentials::password("pi", "hunter2".into()).unwrap();
        let text = format!("{credentials:?}");
        assert!(text.contains("pi"));
        assert!(!text.contains("hunter2"));
    }
}
