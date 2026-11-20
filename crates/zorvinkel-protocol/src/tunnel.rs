//! Terminal tunnel over Bluetooth (ADR-031): messages, record contents and flow control.
//!
//! The tunnel carries a byte stream to the device's local `sshd`. It lives next to the status
//! session: the session opens it, the tunnel then runs under its own keys. This module holds only
//! what needs no cryptography or radio: the bounded wire formats and the credit window.
use crate::status::CodecError;

/// The device can open a tunnel to its local `sshd` (ADR-031).
pub const CAP_TUNNEL: u32 = 4096;

/// A record is carried in one notification or write, never fragmented: smallest and largest
/// frame length the two sides may agree on. 185 bytes was the best measured on a Raspberry Pi 4
/// (docs/validation/Terminal-Tunnel.md); below 64 bytes a record has no room for data.
pub const MIN_FRAME: usize = 64;
pub const MAX_FRAME: usize = 185;
/// Frames the sender may have in flight. Windows keeps 16 notifications per peripheral, and
/// BlueZ queues without bound, so the window protects both interactivity and delivery.
pub const WINDOW_FRAMES: u16 = 16;
/// Length of the authentication tag every record carries.
pub const TAG_LENGTH: usize = 16;
/// Largest number of data bytes one record carries for a given frame length.
pub fn max_data(frame_length: usize) -> usize {
    frame_length.saturating_sub(TAG_LENGTH + 1)
}

/// Size of the key exchange messages inside `OpenRequest` and `OpenResponse` (Noise NN).
pub const REQUEST_NOISE: usize = 32;
pub const RESPONSE_NOISE: usize = 48;

fn frame_length_valid(frame_length: u16) -> bool {
    (MIN_FRAME..=MAX_FRAME).contains(&usize::from(frame_length))
}

/// The client asks for a tunnel: the frame length it can send and receive and its half of the
/// key exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenRequest {
    pub frame_length: u16,
    pub noise: [u8; REQUEST_NOISE],
}
impl OpenRequest {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.frame_length.to_le_bytes().to_vec();
        bytes.extend(self.noise);
        bytes
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() != 2 + REQUEST_NOISE {
            return Err(CodecError::Malformed);
        }
        let frame_length = u16::from_le_bytes([bytes[0], bytes[1]]);
        if !frame_length_valid(frame_length) {
            return Err(CodecError::Malformed);
        }
        Ok(Self {
            frame_length,
            noise: bytes[2..].try_into().map_err(|_| CodecError::Malformed)?,
        })
    }
}

/// The device accepts, possibly with a smaller frame length, and answers the key exchange.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenResponse {
    pub frame_length: u16,
    /// Frames the client may send before it needs credit.
    pub window: u16,
    pub noise: [u8; RESPONSE_NOISE],
}
impl OpenResponse {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = self.frame_length.to_le_bytes().to_vec();
        bytes.extend(self.window.to_le_bytes());
        bytes.extend(self.noise);
        bytes
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, CodecError> {
        if bytes.len() != 4 + RESPONSE_NOISE {
            return Err(CodecError::Malformed);
        }
        let frame_length = u16::from_le_bytes([bytes[0], bytes[1]]);
        let window = u16::from_le_bytes([bytes[2], bytes[3]]);
        if !frame_length_valid(frame_length) || window == 0 || window > WINDOW_FRAMES {
            return Err(CodecError::Malformed);
        }
        Ok(Self {
            frame_length,
            window,
            noise: bytes[4..].try_into().map_err(|_| CodecError::Malformed)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseReason {
    /// The side closing has no more data, or the user ended the terminal.
    Normal,
    /// `sshd` could not be reached or closed its side.
    Backend,
    /// The side closing saw a protocol violation.
    Protocol,
    /// Policy ended the tunnel: lifetime reached or registration no longer valid.
    Policy,
}
impl CloseReason {
    fn code(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Backend => 1,
            Self::Protocol => 2,
            Self::Policy => 3,
        }
    }
    fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            0 => Self::Normal,
            1 => Self::Backend,
            2 => Self::Protocol,
            3 => Self::Policy,
            _ => return None,
        })
    }
}

/// What a record carries once decrypted. The first byte is the kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Record {
    /// Bytes of the stream, in order.
    Data(Vec<u8>),
    /// The receiver freed this many frames of its window.
    Credit(u16),
    Close(CloseReason),
}
const DATA: u8 = 1;
const CREDIT: u8 = 2;
const CLOSE: u8 = 3;

impl Record {
    /// The plaintext of a record for the agreed frame length, `None` when it does not fit.
    pub fn to_plaintext(&self, frame_length: usize) -> Option<Vec<u8>> {
        let bytes = match self {
            Self::Data(data) if !data.is_empty() && data.len() <= max_data(frame_length) => {
                let mut bytes = vec![DATA];
                bytes.extend(data);
                bytes
            }
            Self::Credit(frames) if *frames > 0 => {
                let mut bytes = vec![CREDIT];
                bytes.extend(frames.to_le_bytes());
                bytes
            }
            Self::Close(reason) => vec![CLOSE, reason.code()],
            _ => return None,
        };
        (bytes.len() <= max_data(frame_length) + 1).then_some(bytes)
    }
    pub fn parse(plaintext: &[u8], frame_length: usize) -> Result<Self, CodecError> {
        let (&kind, body) = plaintext.split_first().ok_or(CodecError::Malformed)?;
        if body.len() > max_data(frame_length) {
            return Err(CodecError::TooLarge);
        }
        match (kind, body) {
            (DATA, data) if !data.is_empty() => Ok(Self::Data(data.to_vec())),
            (CREDIT, [low, high]) => match u16::from_le_bytes([*low, *high]) {
                0 => Err(CodecError::Malformed),
                frames => Ok(Self::Credit(frames)),
            },
            (CLOSE, [code]) => CloseReason::from_code(*code)
                .map(Self::Close)
                .ok_or(CodecError::Malformed),
            _ => Err(CodecError::Malformed),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowError {
    /// The peer sent more frames than it had credit for.
    Overrun,
    /// The peer granted more credit than frames it could have received.
    CreditOverflow,
}

/// Credit of the sending side: how many frames may still be sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendWindow {
    capacity: u16,
    available: u16,
}
impl SendWindow {
    pub fn new(capacity: u16) -> Self {
        Self {
            capacity,
            available: capacity,
        }
    }
    pub fn available(&self) -> u16 {
        self.available
    }
    /// Takes the credit of one frame; `false` when the sender has to wait.
    pub fn take(&mut self) -> bool {
        match self.available.checked_sub(1) {
            Some(rest) => {
                self.available = rest;
                true
            }
            None => false,
        }
    }
    /// A peer granting more than was ever taken is faulty: the window would exceed its size.
    pub fn grant(&mut self, frames: u16) -> Result<(), WindowError> {
        let total = self
            .available
            .checked_add(frames)
            .filter(|total| *total <= self.capacity)
            .ok_or(WindowError::CreditOverflow)?;
        self.available = total;
        Ok(())
    }
}

/// Credit of the receiving side: counts frames and says when to give credit back. Credit goes
/// out in batches of half a window, so a steady stream does not double the number of frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReceiveWindow {
    capacity: u16,
    in_flight: u16,
    consumed: u16,
}
impl ReceiveWindow {
    pub fn new(capacity: u16) -> Self {
        Self {
            capacity,
            in_flight: 0,
            consumed: 0,
        }
    }
    /// A data frame arrived. A peer that exceeds the window is faulty.
    pub fn arrived(&mut self) -> Result<(), WindowError> {
        if self.in_flight >= self.capacity {
            return Err(WindowError::Overrun);
        }
        self.in_flight += 1;
        Ok(())
    }
    /// The application took a frame out of the receive queue. Returns the credit to send now,
    /// once half a window has been freed.
    pub fn consumed(&mut self) -> Option<u16> {
        self.in_flight = self.in_flight.saturating_sub(1);
        self.consumed += 1;
        if self.consumed >= self.capacity.div_ceil(2) {
            return Some(std::mem::take(&mut self.consumed));
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_messages_round_trip() {
        let request = OpenRequest {
            frame_length: 185,
            noise: [7; REQUEST_NOISE],
        };
        assert_eq!(OpenRequest::parse(&request.to_bytes()), Ok(request));
        let response = OpenResponse {
            frame_length: 128,
            window: 16,
            noise: [9; RESPONSE_NOISE],
        };
        assert_eq!(OpenResponse::parse(&response.to_bytes()), Ok(response));
    }
    #[test]
    fn open_messages_reject_bad_sizes_and_values() {
        let mut request = OpenRequest {
            frame_length: 185,
            noise: [0; REQUEST_NOISE],
        }
        .to_bytes();
        assert!(OpenRequest::parse(&request[..request.len() - 1]).is_err());
        request[..2].copy_from_slice(&20u16.to_le_bytes());
        assert!(OpenRequest::parse(&request).is_err(), "below the minimum");
        request[..2].copy_from_slice(&512u16.to_le_bytes());
        assert!(OpenRequest::parse(&request).is_err(), "above the maximum");
        let mut response = OpenResponse {
            frame_length: 185,
            window: 16,
            noise: [0; RESPONSE_NOISE],
        }
        .to_bytes();
        response[2..4].copy_from_slice(&0u16.to_le_bytes());
        assert!(OpenResponse::parse(&response).is_err(), "no window");
        response[2..4].copy_from_slice(&17u16.to_le_bytes());
        assert!(OpenResponse::parse(&response).is_err(), "window too large");
    }

    #[test]
    fn tunnel_open_travels_as_a_status_operation() {
        use crate::status::*;
        let open = OpenRequest {
            frame_length: 185,
            noise: [3; REQUEST_NOISE],
        };
        let request = Request {
            id: 5,
            operation: Operation::TunnelOpen,
        };
        let bytes = encode_request_payload(request, &open.to_bytes()).unwrap();
        assert_eq!(decode_request(&bytes), Ok(request));
        assert_eq!(request_payload(&bytes).unwrap(), open.to_bytes());
        assert!(
            encode_request_payload(request, &[]).is_err(),
            "needs the key exchange"
        );
        let response = Response {
            id: 5,
            body: ResponseBody::TunnelOpened(OpenResponse {
                frame_length: 185,
                window: 16,
                noise: [4; RESPONSE_NOISE],
            }),
        };
        assert_eq!(
            decode_response(&encode_response(&response).unwrap()),
            Ok(response)
        );
    }

    #[test]
    fn records_round_trip_within_the_frame_length() {
        let longest = vec![0x42; max_data(185)];
        for record in [
            Record::Data(vec![1]),
            Record::Data(longest),
            Record::Credit(8),
            Record::Close(CloseReason::Policy),
        ] {
            let plaintext = record.to_plaintext(185).unwrap();
            assert_eq!(Record::parse(&plaintext, 185), Ok(record));
        }
    }
    #[test]
    fn records_that_do_not_fit_or_make_no_sense_are_refused() {
        assert_eq!(
            Record::Data(vec![0; max_data(185) + 1]).to_plaintext(185),
            None
        );
        assert_eq!(Record::Data(Vec::new()).to_plaintext(185), None);
        assert_eq!(Record::Credit(0).to_plaintext(185), None);
        assert_eq!(Record::parse(&[], 185), Err(CodecError::Malformed));
        assert_eq!(Record::parse(&[DATA], 185), Err(CodecError::Malformed));
        assert_eq!(
            Record::parse(&[CREDIT, 0, 0], 185),
            Err(CodecError::Malformed)
        );
        assert_eq!(Record::parse(&[CLOSE, 9], 185), Err(CodecError::Malformed));
        assert_eq!(Record::parse(&[0x7F, 1], 185), Err(CodecError::Malformed));
        let too_long = [vec![DATA], vec![0; max_data(185) + 1]].concat();
        assert_eq!(Record::parse(&too_long, 185), Err(CodecError::TooLarge));
    }
    #[test]
    fn a_smaller_agreed_frame_length_limits_the_data() {
        assert_eq!(max_data(64), 47);
        assert!(Record::Data(vec![0; 47]).to_plaintext(64).is_some());
        assert!(Record::Data(vec![0; 48]).to_plaintext(64).is_none());
    }

    #[test]
    fn the_sender_stops_at_the_end_of_its_window_and_continues_with_credit() {
        let mut window = SendWindow::new(4);
        assert!((0..4).all(|_| window.take()));
        assert!(!window.take());
        assert_eq!(window.grant(2), Ok(()));
        assert_eq!(window.available(), 2);
        assert!(window.take() && window.take() && !window.take());
    }
    #[test]
    fn credit_beyond_the_window_is_a_protocol_error() {
        let mut window = SendWindow::new(4);
        assert_eq!(window.grant(1), Err(WindowError::CreditOverflow));
        assert!(window.take());
        assert_eq!(window.grant(1), Ok(()));
        assert_eq!(window.grant(1), Err(WindowError::CreditOverflow));
    }
    #[test]
    fn the_receiver_gives_credit_back_in_half_windows() {
        let mut window = ReceiveWindow::new(16);
        for _ in 0..16 {
            assert_eq!(window.arrived(), Ok(()));
        }
        assert_eq!(window.arrived(), Err(WindowError::Overrun));
        let credits: Vec<_> = (0..16).filter_map(|_| window.consumed()).collect();
        assert_eq!(credits, [8, 8]);
        assert_eq!(window.arrived(), Ok(()));
    }
    #[test]
    fn sender_and_receiver_windows_stay_in_balance() {
        let (mut send, mut receive) = (SendWindow::new(16), ReceiveWindow::new(16));
        let mut delivered = 0u32;
        for _ in 0..100 {
            let mut in_flight = 0;
            while send.take() {
                receive.arrived().unwrap();
                in_flight += 1;
                delivered += 1;
            }
            assert_eq!(in_flight, 16, "a full window every round");
            // The application reads every waiting frame; credit comes back in batches.
            for _ in 0..in_flight {
                if let Some(credit) = receive.consumed() {
                    send.grant(credit).unwrap();
                }
            }
            assert_eq!(send.available(), 16);
        }
        assert_eq!(delivered, 1600);
    }
}
