//! Dedicated test UUIDs; these endpoints never dispatch application commands.
pub const SERVICE_UUID: u128 = 0x719e40a0_4a4b_4d75_9c08_a9cbb88f4100;
pub const WRITE_UUID: u128 = SERVICE_UUID + 1;
pub const NOTIFY_UUID: u128 = SERVICE_UUID + 2;
pub const REQUEST: [u8; 256] = pattern(1);
pub const RESPONSE: [u8; 256] = pattern(2);
const fn pattern(marker: u8) -> [u8; 256] {
    let mut bytes = [0; 256];
    let mut index = 0;
    while index < bytes.len() {
        bytes[index] = index as u8;
        index += 1;
    }
    bytes[0] = marker;
    bytes
}

pub fn respond(request: &[u8]) -> Option<&'static [u8]> {
    (request == REQUEST).then_some(&RESPONSE)
}
