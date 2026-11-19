use std::fmt;
use zeroize::Zeroizing;

const MAX_USER_LENGTH: usize = 64;
const MAX_PASSWORD_LENGTH: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidCredentials {
    User,
    Password,
}

/// Operating-system account of the device. The password stays in memory for the duration of the
/// terminal only (never stored) and is cleared when dropped.
#[derive(Clone)]
pub struct Credentials {
    user: String,
    password: Zeroizing<String>,
}
impl Credentials {
    pub fn new(user: &str, password: String) -> Result<Self, InvalidCredentials> {
        if user.is_empty()
            || user.len() > MAX_USER_LENGTH
            || user.chars().any(|c| c.is_control() || c.is_whitespace())
        {
            return Err(InvalidCredentials::User);
        }
        if password.len() > MAX_PASSWORD_LENGTH {
            return Err(InvalidCredentials::Password);
        }
        Ok(Self {
            user: user.to_owned(),
            password: Zeroizing::new(password),
        })
    }
    pub fn user(&self) -> &str {
        &self.user
    }
    pub(crate) fn password(&self) -> &str {
        &self.password
    }
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
                Credentials::new(user, "secret".into()).err(),
                Some(InvalidCredentials::User)
            );
        }
    }
    #[test]
    fn rejects_oversized_password() {
        assert_eq!(
            Credentials::new("pi", "x".repeat(1025)).err(),
            Some(InvalidCredentials::Password)
        );
    }
    #[test]
    fn debug_output_hides_the_password() {
        let credentials = Credentials::new("pi", "hunter2".into()).unwrap();
        let text = format!("{credentials:?}");
        assert!(text.contains("pi"));
        assert!(!text.contains("hunter2"));
    }
}
