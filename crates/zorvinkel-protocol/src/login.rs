//! Registration with the operating-system account of the device (ADR-037). The request carries
//! the user name and the password inside the secured session; nothing here is logged or
//! printed, and the types of this module have no `Debug` form that shows the password.
use crate::status::CodecError;

pub const USER_MAX: usize = 32;
pub const PASSWORD_MAX: usize = 512;

/// The user name of a Linux account: what `useradd` accepts, without the rarer forms.
pub fn valid_user(user: &str) -> bool {
    let mut characters = user.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    user.len() <= USER_MAX
        && (first.is_ascii_lowercase() || first == '_')
        && characters.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-.$".contains(c))
}
/// Any text without control characters; the operating system decides whether it is right.
pub fn valid_password(password: &str) -> bool {
    !password.is_empty()
        && password.len() <= PASSWORD_MAX
        && !password.chars().any(char::is_control)
}

/// `user length (1 byte) | user | password length (2 bytes, little endian) | password`.
pub fn login_payload(user: &str, password: &str) -> Result<Vec<u8>, CodecError> {
    if !valid_user(user) || !valid_password(password) {
        return Err(CodecError::Malformed);
    }
    let mut payload = Vec::with_capacity(3 + user.len() + password.len());
    payload.push(user.len() as u8);
    payload.extend(user.as_bytes());
    payload.extend((password.len() as u16).to_le_bytes());
    payload.extend(password.as_bytes());
    Ok(payload)
}

/// The user name and the password, borrowed from the payload so that no second copy is made.
pub fn read_login(payload: &[u8]) -> Result<(&str, &str), CodecError> {
    let (&user_length, rest) = payload.split_first().ok_or(CodecError::Malformed)?;
    let user_length = usize::from(user_length);
    if rest.len() < user_length + 2 {
        return Err(CodecError::Malformed);
    }
    let (user, rest) = rest.split_at(user_length);
    let (password_length, password) = rest.split_at(2);
    let password_length = usize::from(u16::from_le_bytes([password_length[0], password_length[1]]));
    if password.len() != password_length {
        return Err(CodecError::Malformed);
    }
    let user = std::str::from_utf8(user).map_err(|_| CodecError::Malformed)?;
    let password = std::str::from_utf8(password).map_err(|_| CodecError::Malformed)?;
    if !valid_user(user) || !valid_password(password) {
        return Err(CodecError::Malformed);
    }
    Ok((user, password))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_round_trips() {
        let payload = login_payload("pi", "pässword with spaces").unwrap();
        assert_eq!(read_login(&payload), Ok(("pi", "pässword with spaces")));
    }
    #[test]
    fn user_names_follow_the_rules_of_linux_accounts() {
        for user in ["pi", "_apt", "a-b.c", "user1", "machine$"] {
            assert!(valid_user(user), "{user}");
        }
        for user in ["", "-x", "Pi", "1pi", "a b", "a\nb", "ü", &"u".repeat(33)] {
            assert!(!valid_user(user), "{user:?}");
        }
    }
    #[test]
    fn passwords_are_bounded_and_free_of_control_characters() {
        assert!(login_payload("pi", "").is_err());
        assert!(login_payload("pi", "a\nb").is_err());
        assert!(login_payload("pi", "a\0b").is_err());
        assert!(login_payload("pi", &"x".repeat(PASSWORD_MAX)).is_ok());
        assert!(login_payload("pi", &"x".repeat(PASSWORD_MAX + 1)).is_err());
    }
    #[test]
    fn truncated_padded_and_invalid_payloads_are_rejected() {
        let payload = login_payload("pi", "secret").unwrap();
        assert!(read_login(&[]).is_err());
        assert!(read_login(&payload[..payload.len() - 1]).is_err());
        let mut padded = payload.clone();
        padded.push(b'x');
        assert!(read_login(&padded).is_err());
        let mut bad_user = payload.clone();
        bad_user[1] = b'P';
        assert!(read_login(&bad_user).is_err());
        let mut bad_text = payload;
        let last = bad_text.len() - 1;
        bad_text[last] = 0xff;
        assert!(read_login(&bad_text).is_err());
    }
}
