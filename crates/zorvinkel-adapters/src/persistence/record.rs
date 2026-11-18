use zorvinkel_application::state::StateError;

pub(super) fn encode(header: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut bytes = header.to_vec();
    bytes.extend(payload);
    bytes.extend(checksum(&bytes).to_le_bytes());
    bytes
}

pub(super) fn decode<'a>(header: &[u8], bytes: &'a [u8]) -> Result<&'a [u8], StateError> {
    let end = bytes.len().checked_sub(4).ok_or(StateError::Corrupt)?;
    if checksum(&bytes[..end]).to_le_bytes() != bytes[end..] {
        return Err(StateError::Corrupt);
    }
    let magic_length = header.len() - 1;
    if bytes.get(..magic_length) != Some(&header[..magic_length]) {
        return Err(StateError::Corrupt);
    }
    if bytes.get(magic_length) != header.get(magic_length) {
        return Err(StateError::UnsupportedVersion);
    }
    bytes.get(header.len()..end).ok_or(StateError::Corrupt)
}

// CRC-32 detects accidental damage; filesystem permissions provide the trust boundary.
pub(super) fn checksum(bytes: &[u8]) -> u32 {
    let mut value = !0_u32;
    for byte in bytes {
        value ^= u32::from(*byte);
        for _ in 0..8 {
            value = (value >> 1) ^ (0xedb88320 & (0_u32.wrapping_sub(value & 1)));
        }
    }
    !value
}
