//! Keys and records of the terminal tunnel (ADR-031).
//!
//! The tunnel runs under its own Noise transport state. The key exchange (`Noise_NN`) travels
//! inside the authenticated status session, so it needs no identities of its own: whoever could
//! tamper with it would have to break that session first. Both directions rekey after a fixed
//! number of records instead of ending at a record limit; the counters are known to both sides,
//! so no message is needed for it.
use super::SecureError;
use std::time::Duration;
use zorvinkel_protocol::tunnel::{
    OpenRequest, OpenResponse, REQUEST_NOISE, RESPONSE_NOISE, Record, TAG_LENGTH,
};

const PATTERN: &str = "Noise_NN_25519_ChaChaPoly_SHA256";
const PROLOGUE: &[u8] = b"zorvinkel/tunnel/1.0";
/// Records per direction between two rekeys. The device may re-check the client's registration
/// at these points ([`TunnelCipher::at_rekey_boundary`]).
pub const REKEY_INTERVAL: u64 = 1024;

fn builder() -> Result<snow::Builder<'static>, SecureError> {
    Ok(snow::Builder::new(PATTERN.parse().map_err(|_| SecureError::Crypto)?).prologue(PROLOGUE)?)
}

/// The client's side between sending the request and receiving the response.
pub struct PendingTunnel {
    state: snow::HandshakeState,
    frame_length: u16,
}

/// The client builds its half of the key exchange and the request to send.
pub fn request(frame_length: u16) -> Result<(PendingTunnel, OpenRequest), SecureError> {
    let mut state = builder()?.build_initiator()?;
    // Noise wants room for a tag even when the message has no payload.
    let mut buffer = [0; 96];
    let length = state.write_message(&[], &mut buffer)?;
    let noise = buffer
        .get(..REQUEST_NOISE)
        .filter(|_| length == REQUEST_NOISE)
        .and_then(|bytes| <[u8; REQUEST_NOISE]>::try_from(bytes).ok())
        .ok_or(SecureError::InvalidMessage)?;
    Ok((
        PendingTunnel {
            state,
            frame_length,
        },
        OpenRequest {
            frame_length,
            noise,
        },
    ))
}

/// The device answers a request with the frame length it accepts and the cipher it will use.
pub fn accept(
    request: &OpenRequest,
    frame_length: u16,
    window: u16,
    now: Duration,
    lifetime: Duration,
) -> Result<(TunnelCipher, OpenResponse), SecureError> {
    if frame_length > request.frame_length {
        return Err(SecureError::InvalidMessage);
    }
    let mut state = builder()?.build_responder()?;
    state.read_message(&request.noise, &mut [])?;
    let mut buffer = [0; 96];
    let length = state.write_message(&[], &mut buffer)?;
    let noise = buffer
        .get(..RESPONSE_NOISE)
        .filter(|_| length == RESPONSE_NOISE)
        .and_then(|bytes| <[u8; RESPONSE_NOISE]>::try_from(bytes).ok())
        .ok_or(SecureError::InvalidMessage)?;
    let cipher = TunnelCipher::new(state.into_transport_mode()?, frame_length, now, lifetime);
    Ok((
        cipher,
        OpenResponse {
            frame_length,
            window,
            noise,
        },
    ))
}

impl PendingTunnel {
    /// The client finishes the key exchange with the device's response.
    pub fn finish(
        mut self,
        response: &OpenResponse,
        now: Duration,
        lifetime: Duration,
    ) -> Result<TunnelCipher, SecureError> {
        if response.frame_length > self.frame_length {
            return Err(SecureError::InvalidMessage);
        }
        self.state.read_message(&response.noise, &mut [])?;
        Ok(TunnelCipher::new(
            self.state.into_transport_mode()?,
            response.frame_length,
            now,
            lifetime,
        ))
    }
}

/// Seals and opens the records of one tunnel. Any error closes it: a lost, repeated or altered
/// record cannot be told apart from an attack, and the client reconnects anyway (ADR-030).
pub struct TunnelCipher {
    state: Option<snow::TransportState>,
    frame_length: usize,
    started: Duration,
    last_time: Duration,
    lifetime: Duration,
    sent: u64,
    received: u64,
}
impl TunnelCipher {
    fn new(
        state: snow::TransportState,
        frame_length: u16,
        now: Duration,
        lifetime: Duration,
    ) -> Self {
        Self {
            state: Some(state),
            frame_length: usize::from(frame_length),
            started: now,
            last_time: now,
            lifetime,
            sent: 0,
            received: 0,
        }
    }
    pub fn frame_length(&self) -> usize {
        self.frame_length
    }
    pub fn is_closed(&self) -> bool {
        self.state.is_none()
    }
    pub fn close(&mut self) {
        self.state = None;
    }
    /// The absolute lifetime has passed; the next record would fail.
    pub fn is_expired(&self, now: Duration) -> bool {
        now.saturating_sub(self.started) >= self.lifetime
    }
    pub fn sent(&self) -> u64 {
        self.sent
    }
    pub fn received(&self) -> u64 {
        self.received
    }
    /// The next record to send or to receive is the first after a rekey: the point where the
    /// device may check that the client is still registered.
    pub fn at_rekey_boundary(&self) -> bool {
        (self.sent > 0 && self.sent.is_multiple_of(REKEY_INTERVAL))
            || (self.received > 0 && self.received.is_multiple_of(REKEY_INTERVAL))
    }
    /// The encrypted frame for `record`, at most the agreed frame length.
    pub fn seal(&mut self, record: &Record, now: Duration) -> Result<Vec<u8>, SecureError> {
        let result = (|| {
            self.check_time(now)?;
            let plaintext = record
                .to_plaintext(self.frame_length)
                .ok_or(SecureError::InvalidMessage)?;
            let state = self.state.as_mut().ok_or(SecureError::Closed)?;
            let mut frame = vec![0; plaintext.len() + TAG_LENGTH];
            let length = state.write_message(&plaintext, &mut frame)?;
            frame.truncate(length);
            self.sent += 1;
            if self.sent.is_multiple_of(REKEY_INTERVAL) {
                state.rekey_outgoing();
            }
            Ok(frame)
        })();
        if result.is_err() {
            self.close();
        }
        result
    }
    pub fn open(&mut self, frame: &[u8], now: Duration) -> Result<Record, SecureError> {
        let result = (|| {
            self.check_time(now)?;
            if frame.len() <= TAG_LENGTH || frame.len() > self.frame_length {
                return Err(SecureError::InvalidMessage);
            }
            let state = self.state.as_mut().ok_or(SecureError::Closed)?;
            let mut plaintext = vec![0; frame.len()];
            let length = state.read_message(frame, &mut plaintext)?;
            plaintext.truncate(length);
            self.received += 1;
            if self.received.is_multiple_of(REKEY_INTERVAL) {
                state.rekey_incoming();
            }
            Ok(Record::parse(&plaintext, self.frame_length)?)
        })();
        if result.is_err() {
            self.close();
        }
        result
    }
    fn check_time(&mut self, now: Duration) -> Result<(), SecureError> {
        if self.state.is_none() {
            return Err(SecureError::Closed);
        }
        if now < self.last_time {
            return Err(SecureError::ClockMovedBackwards);
        }
        self.last_time = now;
        if now - self.started >= self.lifetime {
            return Err(SecureError::TimedOut);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zorvinkel_protocol::tunnel::{CloseReason, max_data};

    const LIFETIME: Duration = Duration::from_secs(8 * 3600);
    const NOW: Duration = Duration::ZERO;

    fn pair(frame_length: u16) -> (TunnelCipher, TunnelCipher) {
        let (pending, open) = request(frame_length).unwrap();
        // The device parses what arrived on the wire, as it would in a real session.
        let open = OpenRequest::parse(&open.to_bytes()).unwrap();
        let (device, response) = accept(&open, frame_length, 16, NOW, LIFETIME).unwrap();
        let response = OpenResponse::parse(&response.to_bytes()).unwrap();
        (pending.finish(&response, NOW, LIFETIME).unwrap(), device)
    }

    #[test]
    fn data_flows_both_ways_and_stays_within_the_frame_length() {
        let (mut client, mut device) = pair(185);
        let record = Record::Data(vec![0x42; max_data(185)]);
        let frame = client.seal(&record, NOW).unwrap();
        assert_eq!(frame.len(), 185, "a full record is exactly one frame");
        assert_eq!(device.open(&frame, NOW), Ok(record));
        let credit = device.seal(&Record::Credit(8), NOW).unwrap();
        assert_eq!(client.open(&credit, NOW), Ok(Record::Credit(8)));
        let close = client
            .seal(&Record::Close(CloseReason::Normal), NOW)
            .unwrap();
        assert_eq!(
            device.open(&close, NOW),
            Ok(Record::Close(CloseReason::Normal))
        );
    }
    #[test]
    fn the_device_may_choose_a_smaller_frame_length_but_not_a_larger_one() {
        let (pending, open) = request(185).unwrap();
        let (_, response) = accept(&open, 100, 16, NOW, LIFETIME).unwrap();
        assert_eq!(response.frame_length, 100);
        assert!(accept(&open, 186, 16, NOW, LIFETIME).is_err());
        let mut larger = response.clone();
        larger.frame_length = 185;
        let (small_pending, _) = request(100).unwrap();
        assert!(small_pending.finish(&larger, NOW, LIFETIME).is_err());
        assert!(pending.finish(&response, NOW, LIFETIME).is_ok());
    }
    #[test]
    fn records_that_do_not_fit_are_refused_and_close_the_cipher() {
        let (mut client, _) = pair(64);
        let too_long = Record::Data(vec![0; max_data(64) + 1]);
        assert_eq!(
            client.seal(&too_long, NOW),
            Err(SecureError::InvalidMessage)
        );
        assert!(client.is_closed());
    }
    #[test]
    fn an_altered_lost_or_repeated_frame_closes_the_tunnel() {
        let (mut client, mut device) = pair(185);
        let first = client.seal(&Record::Data(vec![1]), NOW).unwrap();
        let second = client.seal(&Record::Data(vec![2]), NOW).unwrap();
        // A lost first frame: the second one does not fit the counter any more.
        assert_eq!(device.open(&second, NOW), Err(SecureError::Crypto));
        assert!(device.is_closed());
        assert_eq!(device.open(&first, NOW), Err(SecureError::Closed));

        let (mut client, mut device) = pair(185);
        let mut frame = client.seal(&Record::Data(vec![1]), NOW).unwrap();
        frame[0] ^= 1;
        assert_eq!(device.open(&frame, NOW), Err(SecureError::Crypto));

        let (mut client, mut device) = pair(185);
        let frame = client.seal(&Record::Data(vec![1]), NOW).unwrap();
        device.open(&frame, NOW).unwrap();
        assert_eq!(device.open(&frame, NOW), Err(SecureError::Crypto), "replay");
    }
    #[test]
    fn frames_of_the_wrong_size_are_refused() {
        let (_, mut device) = pair(100);
        assert_eq!(device.open(&[0; 16], NOW), Err(SecureError::InvalidMessage));
        let (_, mut device) = pair(100);
        assert_eq!(
            device.open(&[0; 101], NOW),
            Err(SecureError::InvalidMessage)
        );
    }
    #[test]
    fn both_directions_keep_working_across_many_rekeys() {
        let (mut client, mut device) = pair(185);
        for round in 0..(3 * REKEY_INTERVAL + 5) {
            let upstream = Record::Data(round.to_le_bytes().to_vec());
            let frame = client.seal(&upstream, NOW).unwrap();
            assert_eq!(device.open(&frame, NOW), Ok(upstream));
            // The other direction is used less often and rekeys at its own record count.
            if round % 3 == 0 {
                let downstream = Record::Data(vec![round as u8]);
                let frame = device.seal(&downstream, NOW).unwrap();
                assert_eq!(client.open(&frame, NOW), Ok(downstream));
            }
        }
        assert!(client.sent() > 3 * REKEY_INTERVAL);
        assert!(device.sent() > REKEY_INTERVAL);
    }
    #[test]
    fn the_rekey_boundary_is_reported_after_the_last_record_before_it() {
        let (mut client, mut device) = pair(185);
        for _ in 0..REKEY_INTERVAL - 1 {
            let frame = client.seal(&Record::Data(vec![0]), NOW).unwrap();
            device.open(&frame, NOW).unwrap();
        }
        assert!(!client.at_rekey_boundary() && !device.at_rekey_boundary());
        let frame = client.seal(&Record::Data(vec![0]), NOW).unwrap();
        assert!(client.at_rekey_boundary());
        device.open(&frame, NOW).unwrap();
        assert!(device.at_rekey_boundary());
    }
    #[test]
    fn a_rekeyed_stream_does_not_accept_frames_of_the_old_key() {
        let (mut client, mut device) = pair(185);
        let mut early = Vec::new();
        for _ in 0..REKEY_INTERVAL {
            let frame = client.seal(&Record::Data(vec![7]), NOW).unwrap();
            early.push(frame.clone());
            device.open(&frame, NOW).unwrap();
        }
        // Replaying a frame from before the rekey fails: the nonce and the key have moved on.
        assert_eq!(device.open(&early[0], NOW), Err(SecureError::Crypto));
    }
    #[test]
    fn the_tunnel_ends_at_its_lifetime_and_with_a_clock_that_runs_backwards() {
        let (mut client, _) = pair(185);
        let record = Record::Data(vec![1]);
        assert!(client.seal(&record, Duration::from_secs(10)).is_ok());
        assert_eq!(
            client.seal(&record, Duration::from_secs(5)),
            Err(SecureError::ClockMovedBackwards)
        );
        let (mut client, _) = pair(185);
        assert_eq!(client.seal(&record, LIFETIME), Err(SecureError::TimedOut));
        assert!(client.is_closed());
    }
    #[test]
    fn two_tunnels_do_not_share_keys() {
        let (mut first_client, _) = pair(185);
        let (_, mut other_device) = pair(185);
        let frame = first_client.seal(&Record::Data(vec![1]), NOW).unwrap();
        assert_eq!(other_device.open(&frame, NOW), Err(SecureError::Crypto));
    }
}
