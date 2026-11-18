use std::time::Duration;

pub const HEADER_LENGTH: usize = 12;
pub const BASELINE_FRAME_LENGTH: usize = 20;
pub const MAX_FRAME_LENGTH: usize = 512;
pub const MAX_MESSAGE_LENGTH: usize = 4096;
pub const MESSAGE_TIMEOUT: Duration = Duration::from_secs(30);
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum MessageKind {
    Probe = 0,
    SecureRecord = 1,
}
const MAGIC: [u8; 2] = *b"ZV";
pub const MAJOR_VERSION: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameError {
    InvalidFrameSize,
    Malformed,
    UnsupportedVersion,
    UnsupportedKind,
    InvalidMessageId,
    MessageTooLarge,
    UnexpectedFragment,
    TimedOut,
    ClockMovedBackwards,
    Closed,
}

/// Fragment one complete probe message. IDs start at one and never wrap within a connection.
pub fn fragment(id: u32, message: &[u8], frame_length: usize) -> Result<Vec<Vec<u8>>, FrameError> {
    fragment_with_kind(id, message, frame_length, MessageKind::Probe)
}

pub fn fragment_with_kind(
    id: u32,
    message: &[u8],
    frame_length: usize,
    kind: MessageKind,
) -> Result<Vec<Vec<u8>>, FrameError> {
    validate_frame_length(frame_length)?;
    if id == 0 {
        return Err(FrameError::InvalidMessageId);
    }
    if message.is_empty() || message.len() > MAX_MESSAGE_LENGTH {
        return Err(FrameError::MessageTooLarge);
    }
    let payload_length = frame_length - HEADER_LENGTH;
    Ok(message
        .chunks(payload_length)
        .enumerate()
        .map(|(index, chunk)| {
            let mut frame = MAGIC.to_vec();
            frame.extend([MAJOR_VERSION, kind as u8]);
            frame.extend(id.to_le_bytes());
            frame.extend((message.len() as u16).to_le_bytes());
            frame.extend(((index * payload_length) as u16).to_le_bytes());
            frame.extend(chunk);
            frame
        })
        .collect())
}

fn validate_frame_length(length: usize) -> Result<(), FrameError> {
    if !(BASELINE_FRAME_LENGTH..=MAX_FRAME_LENGTH).contains(&length) {
        return Err(FrameError::InvalidFrameSize);
    }
    Ok(())
}

struct PartialMessage {
    id: u32,
    length: usize,
    started: Duration,
    bytes: Vec<u8>,
}

/// One receiver per connection and direction. Any error closes it until a new connection.
/// Time is monotonic elapsed time supplied by the runtime; tests can control it.
pub struct Reassembler {
    frame_length: usize,
    kind: MessageKind,
    last_id: u32,
    last_time: Duration,
    partial: Option<PartialMessage>,
    closed: bool,
}
impl Reassembler {
    pub fn new(frame_length: usize) -> Result<Self, FrameError> {
        Self::with_kind(frame_length, MessageKind::Probe)
    }
    pub fn with_kind(frame_length: usize, kind: MessageKind) -> Result<Self, FrameError> {
        validate_frame_length(frame_length)?;
        Ok(Self {
            frame_length,
            kind,
            last_id: 0,
            last_time: Duration::ZERO,
            partial: None,
            closed: false,
        })
    }

    pub fn receive(&mut self, frame: &[u8], now: Duration) -> Result<Option<Vec<u8>>, FrameError> {
        let result = self.receive_inner(frame, now);
        if result.is_err() {
            self.close();
        }
        result
    }

    pub fn poll_timeout(&mut self, now: Duration) -> Result<(), FrameError> {
        let result = self.check_time(now);
        if result.is_err() {
            self.close();
        }
        result
    }

    pub fn close(&mut self) {
        self.partial = None;
        self.closed = true;
    }

    fn check_time(&mut self, now: Duration) -> Result<(), FrameError> {
        if self.closed {
            return Err(FrameError::Closed);
        }
        if now < self.last_time {
            return Err(FrameError::ClockMovedBackwards);
        }
        self.last_time = now;
        if self
            .partial
            .as_ref()
            .is_some_and(|partial| now.saturating_sub(partial.started) >= MESSAGE_TIMEOUT)
        {
            return Err(FrameError::TimedOut);
        }
        Ok(())
    }

    fn receive_inner(
        &mut self,
        frame: &[u8],
        now: Duration,
    ) -> Result<Option<Vec<u8>>, FrameError> {
        self.check_time(now)?;
        if frame.len() <= HEADER_LENGTH || frame.len() > self.frame_length {
            return Err(FrameError::Malformed);
        }
        if frame[..2] != MAGIC {
            return Err(FrameError::Malformed);
        }
        if frame[2] != MAJOR_VERSION {
            return Err(FrameError::UnsupportedVersion);
        }
        if frame[3] != self.kind as u8 {
            return Err(FrameError::UnsupportedKind);
        }
        let id = u32::from_le_bytes(frame[4..8].try_into().map_err(|_| FrameError::Malformed)?);
        let length = u16::from_le_bytes(frame[8..10].try_into().map_err(|_| FrameError::Malformed)?)
            as usize;
        let offset = u16::from_le_bytes(
            frame[10..12]
                .try_into()
                .map_err(|_| FrameError::Malformed)?,
        ) as usize;
        if length == 0 || length > MAX_MESSAGE_LENGTH {
            return Err(FrameError::MessageTooLarge);
        }
        if id == 0 || self.last_id.checked_add(1) != Some(id) {
            return Err(FrameError::InvalidMessageId);
        }
        if self.partial.is_none() {
            if offset != 0 {
                return Err(FrameError::UnexpectedFragment);
            }
            self.partial = Some(PartialMessage {
                id,
                length,
                started: now,
                bytes: Vec::with_capacity(length),
            });
        }
        let partial = self.partial.as_mut().ok_or(FrameError::Closed)?;
        if partial.id != id
            || partial.length != length
            || partial.bytes.len() != offset
            || offset + frame.len() - HEADER_LENGTH > length
        {
            return Err(FrameError::UnexpectedFragment);
        }
        partial.bytes.extend_from_slice(&frame[HEADER_LENGTH..]);
        if partial.bytes.len() == length {
            self.last_id = id;
            return Ok(self.partial.take().map(|partial| partial.bytes));
        }
        Ok(None)
    }
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "BLE framing error: {self:?}")
    }
}
impl std::error::Error for FrameError {}
