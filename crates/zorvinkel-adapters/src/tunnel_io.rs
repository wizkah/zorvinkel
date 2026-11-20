//! Forwards the records of a terminal tunnel (ADR-031) to and from a byte stream.
//!
//! The same loop runs on both ends. On the device the stream is the loopback connection to
//! `sshd`; on the client it is one end of an in-memory pipe whose other end is handed to the SSH
//! library. The transport underneath (Bluetooth characteristic, simulation) only moves opaque
//! frames through two channels, so the loop knows neither radio nor sockets.
use crate::security::{SecureError, tunnel::TunnelCipher};
use std::time::Duration;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::mpsc,
    time::{Instant, MissedTickBehavior, interval, timeout},
};
use zorvinkel_protocol::tunnel::{CloseReason, ReceiveWindow, Record, SendWindow, max_data};

/// How often the loop looks at the policy (registration, lifetime) while nothing happens.
const POLL: Duration = Duration::from_secs(1);
/// How long queued bytes may take to reach the stream after the peer closed normally.
const DRAIN: Duration = Duration::from_secs(5);

/// The frames of one tunnel as the transport delivers and takes them.
pub struct FrameLink {
    pub incoming: mpsc::Receiver<Vec<u8>>,
    pub outgoing: mpsc::Sender<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    /// The stream ended on this side.
    Stream,
    /// The stream failed on this side.
    Backend,
    /// The peer closed the tunnel.
    Peer(CloseReason),
    /// A record was invalid, or a peer broke the window.
    Protocol,
    /// Lifetime reached or registration no longer valid.
    Policy,
    /// The transport under the tunnel is gone.
    Transport,
}

/// Runs until the tunnel ends and returns why. `clock` is the time base of the cipher;
/// `allowed` is asked about once a second and at every rekey: `false` ends the tunnel.
pub async fn forward<S>(
    mut cipher: TunnelCipher,
    window: u16,
    mut link: FrameLink,
    stream: S,
    clock: impl Fn() -> Duration,
    mut allowed: impl FnMut() -> bool,
) -> End
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (mut reader, writer) = tokio::io::split(stream);
    let (to_writer, writer_input) = mpsc::channel::<Vec<u8>>(usize::from(window));
    let (consumed_sender, mut consumed) = mpsc::unbounded_channel::<()>();
    let writer_task = tokio::spawn(write_stream(writer, writer_input, consumed_sender));
    let mut sending = SendWindow::new(window);
    let mut receiving = ReceiveWindow::new(window);
    let mut buffer = vec![0; max_data(cipher.frame_length())];
    let mut policy_timer = interval(POLL);
    policy_timer.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let end = loop {
        tokio::select! {
            frame = link.incoming.recv() => {
                let Some(frame) = frame else { break End::Transport };
                let record = match cipher.open(&frame, clock()) {
                    Ok(record) => record,
                    Err(error) => break failure(error),
                };
                match record {
                    Record::Data(data) => {
                        if receiving.arrived().is_err() || to_writer.try_send(data).is_err() {
                            break End::Protocol;
                        }
                    }
                    Record::Credit(frames) => {
                        if sending.grant(frames).is_err() {
                            break End::Protocol;
                        }
                    }
                    Record::Close(reason) => break End::Peer(reason),
                }
                if cipher.at_rekey_boundary() && !allowed() {
                    break End::Policy;
                }
            }
            freed = consumed.recv() => {
                // The writer is gone: the stream failed.
                if freed.is_none() {
                    break End::Backend;
                }
                if let Some(frames) = receiving.consumed()
                    && let Err(end) = send(&mut cipher, &link, &Record::Credit(frames), &clock).await
                {
                    break end;
                }
            }
            read = reader.read(&mut buffer), if sending.available() > 0 => {
                match read {
                    Ok(length) if length > 0 => {
                        sending.take();
                        let record = Record::Data(buffer[..length].to_vec());
                        if let Err(end) = send(&mut cipher, &link, &record, &clock).await {
                            break end;
                        }
                    }
                    Ok(_) => break End::Stream,
                    Err(_) => break End::Backend,
                }
            }
            _ = policy_timer.tick() => {
                if cipher.is_expired(clock()) || !allowed() {
                    break End::Policy;
                }
            }
        }
    };

    let reason = match end {
        // The stream ended on its own: the normal way a shell exits.
        End::Stream => Some(CloseReason::Normal),
        End::Backend => Some(CloseReason::Backend),
        End::Protocol => Some(CloseReason::Protocol),
        End::Policy => Some(CloseReason::Policy),
        // The peer closed, or nothing can be sent any more.
        End::Peer(_) | End::Transport => None,
    };
    if let Some(reason) = reason {
        let _ = send(&mut cipher, &link, &Record::Close(reason), &clock).await;
    }
    cipher.close();
    drop(to_writer);
    // Bytes already accepted still belong to the stream; give them time, then stop.
    if matches!(end, End::Peer(CloseReason::Normal)) {
        let _ = timeout(DRAIN, writer_task).await;
    } else {
        writer_task.abort();
    }
    end
}

fn failure(error: SecureError) -> End {
    match error {
        SecureError::TimedOut => End::Policy,
        _ => End::Protocol,
    }
}

async fn send(
    cipher: &mut TunnelCipher,
    link: &FrameLink,
    record: &Record,
    clock: &impl Fn() -> Duration,
) -> Result<(), End> {
    let frame = cipher.seal(record, clock()).map_err(failure)?;
    link.outgoing.send(frame).await.map_err(|_| End::Transport)
}

/// Writes what arrives to the stream and reports every written chunk, so the loop can return
/// credit. Ends, closing the stream, when the loop drops its sender or a write fails.
async fn write_stream<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut input: mpsc::Receiver<Vec<u8>>,
    consumed: mpsc::UnboundedSender<()>,
) {
    while let Some(chunk) = input.recv().await {
        if writer.write_all(&chunk).await.is_err() || writer.flush().await.is_err() {
            return;
        }
        if consumed.send(()).is_err() {
            break;
        }
    }
    let _ = writer.shutdown().await;
}

/// The loop's own start time as a clock, for callers that have no other time base.
pub fn clock_from_now() -> impl Fn() -> Duration {
    let start = Instant::now();
    move || start.elapsed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::tunnel::{accept, request};
    use tokio::io::{DuplexStream, duplex};
    use zorvinkel_protocol::tunnel::{OpenRequest, WINDOW_FRAMES};

    const LIFETIME: Duration = Duration::from_secs(3600);

    struct Pair {
        client: tokio::task::JoinHandle<End>,
        device: tokio::task::JoinHandle<End>,
        /// What the SSH library holds on the client; what `sshd` holds on the device.
        client_stream: DuplexStream,
        device_stream: DuplexStream,
    }

    fn start(frame_length: u16, device_allowed: impl FnMut() -> bool + Send + 'static) -> Pair {
        let (pending, open) = request(frame_length).unwrap();
        let open = OpenRequest::parse(&open.to_bytes()).unwrap();
        let (device_cipher, response) =
            accept(&open, frame_length, WINDOW_FRAMES, Duration::ZERO, LIFETIME).unwrap();
        let client_cipher = pending.finish(&response, Duration::ZERO, LIFETIME).unwrap();
        let (to_device, device_in) = mpsc::channel(256);
        let (to_client, client_in) = mpsc::channel(256);
        let (client_stream, client_end) = duplex(64 * 1024);
        let (device_stream, device_end) = duplex(64 * 1024);
        let client = tokio::spawn(forward(
            client_cipher,
            WINDOW_FRAMES,
            FrameLink {
                incoming: client_in,
                outgoing: to_device,
            },
            client_end,
            || Duration::ZERO,
            || true,
        ));
        let device = tokio::spawn(forward(
            device_cipher,
            WINDOW_FRAMES,
            FrameLink {
                incoming: device_in,
                outgoing: to_client,
            },
            device_end,
            || Duration::ZERO,
            device_allowed,
        ));
        Pair {
            client,
            device,
            client_stream,
            device_stream,
        }
    }

    fn pattern(length: usize, seed: u8) -> Vec<u8> {
        (0..length)
            .map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    #[tokio::test]
    async fn bulk_data_arrives_intact_in_both_directions() {
        let Pair {
            client,
            device,
            client_stream,
            device_stream,
        } = start(185, || true);
        let (up, down) = (pattern(300_000, 1), pattern(200_000, 2));
        let (mut client_read, mut client_write) = tokio::io::split(client_stream);
        let (mut device_read, mut device_write) = tokio::io::split(device_stream);
        let (up_copy, down_copy) = (up.clone(), down.clone());
        let writers = tokio::spawn(async move {
            client_write.write_all(&up_copy).await.unwrap();
        });
        let device_writer = tokio::spawn(async move {
            device_write.write_all(&down_copy).await.unwrap();
        });
        let (mut at_device, mut at_client) = (vec![0; up.len()], vec![0; down.len()]);
        device_read.read_exact(&mut at_device).await.unwrap();
        client_read.read_exact(&mut at_client).await.unwrap();
        assert_eq!(at_device, up);
        assert_eq!(at_client, down);
        writers.await.unwrap();
        device_writer.await.unwrap();
        drop((device_read, client_read));
        for end in [client.await.unwrap(), device.await.unwrap()] {
            assert!(matches!(end, End::Stream | End::Peer(CloseReason::Normal)));
        }
    }

    #[tokio::test]
    async fn a_stream_that_is_not_read_stops_the_sender_at_the_window() {
        let Pair {
            client,
            device,
            mut client_stream,
            device_stream,
        } = start(185, || true);
        // Nobody reads `device_stream`: its pipe fills, then the writer blocks, then credit
        // stops, then the client stops reading its own stream. Writes into the client's pipe
        // therefore block instead of growing a queue without bound.
        let blocked = tokio::time::timeout(Duration::from_millis(700), async {
            let chunk = pattern(16 * 1024, 3);
            for _ in 0..64 {
                client_stream.write_all(&chunk).await.unwrap();
            }
        })
        .await;
        assert!(
            blocked.is_err(),
            "1 MiB cannot be absorbed by a stalled peer"
        );
        drop(device_stream);
        drop(client_stream);
        let _ = (client.await, device.await);
    }

    #[tokio::test]
    async fn a_policy_refusal_ends_both_sides_with_the_reason() {
        let Pair {
            client,
            device,
            client_stream: _client_stream,
            device_stream: _device_stream,
        } = start(185, || false);
        assert_eq!(device.await.unwrap(), End::Policy);
        assert_eq!(client.await.unwrap(), End::Peer(CloseReason::Policy));
    }

    #[tokio::test]
    async fn a_vanished_transport_ends_the_tunnel() {
        let (pending, open) = request(185).unwrap();
        let (device_cipher, response) =
            accept(&open, 185, WINDOW_FRAMES, Duration::ZERO, LIFETIME).unwrap();
        drop(pending.finish(&response, Duration::ZERO, LIFETIME).unwrap());
        let (to_device, device_in) = mpsc::channel(4);
        let (to_client, client_in) = mpsc::channel(4);
        let (_stream, end) = duplex(1024);
        drop((to_device, client_in));
        let result = forward(
            device_cipher,
            WINDOW_FRAMES,
            FrameLink {
                incoming: device_in,
                outgoing: to_client,
            },
            end,
            || Duration::ZERO,
            || true,
        )
        .await;
        assert_eq!(result, End::Transport);
    }

    #[tokio::test]
    async fn a_forged_frame_ends_the_tunnel_with_a_protocol_error() {
        let (pending, open) = request(185).unwrap();
        let (device_cipher, response) =
            accept(&open, 185, WINDOW_FRAMES, Duration::ZERO, LIFETIME).unwrap();
        drop(pending.finish(&response, Duration::ZERO, LIFETIME).unwrap());
        let (to_device, device_in) = mpsc::channel(4);
        let (to_client, mut client_in) = mpsc::channel(4);
        let (_stream, end) = duplex(1024);
        to_device.send(vec![7; 100]).await.unwrap();
        let result = forward(
            device_cipher,
            WINDOW_FRAMES,
            FrameLink {
                incoming: device_in,
                outgoing: to_client,
            },
            end,
            || Duration::ZERO,
            || true,
        )
        .await;
        assert_eq!(result, End::Protocol);
        // The cipher closed on the failure, so nothing more is sealed: the peer sees the link end.
        assert!(client_in.recv().await.is_none());
    }
}
