use std::time::Duration;
use zorvinkel_adapters::ble::ProbeSession;
use zorvinkel_protocol::{
    ble::{FrameError, MESSAGE_TIMEOUT},
    probe,
};

fn roundtrip(client: &mut ProbeSession, server: &mut ProbeSession) {
    let request_frames = client.send(&probe::REQUEST).unwrap();
    assert!(request_frames.len() > 1);
    let mut replies = Vec::new();
    for frame in request_frames {
        if let Some(message) = server.receive(&frame, Duration::ZERO).unwrap() {
            replies = server.respond(&message).unwrap();
        }
    }
    assert!(replies.len() > 1);
    let mut response = None;
    for frame in replies {
        response = client.receive(&frame, Duration::ZERO).unwrap();
    }
    assert_eq!(response, Some(probe::RESPONSE.to_vec()));
}

#[test]
fn public_probe_roundtrips_both_fragmented_directions() {
    let mut client = ProbeSession::new();
    let mut server = ProbeSession::new();
    roundtrip(&mut client, &mut server);
    roundtrip(&mut client, &mut server);
}

#[test]
fn partial_disconnect_is_discarded_and_reconnect_uses_fresh_state() {
    let mut client = ProbeSession::new();
    let mut server = ProbeSession::new();
    let frames = client.send(&probe::REQUEST).unwrap();
    server.receive(&frames[0], Duration::ZERO).unwrap();
    client.disconnect();
    server.disconnect();
    assert_eq!(
        server.receive(&frames[1], Duration::ZERO),
        Err(FrameError::Closed)
    );
    roundtrip(&mut ProbeSession::new(), &mut ProbeSession::new());
}

#[test]
fn malformed_input_and_timeouts_close_both_directions() {
    let mut session = ProbeSession::new();
    assert!(session.receive(b"bad", Duration::ZERO).is_err());
    assert_eq!(session.send(&probe::REQUEST), Err(FrameError::Closed));
    let frames = ProbeSession::new().send(&probe::REQUEST).unwrap();
    let mut session = ProbeSession::new();
    session.receive(&frames[0], Duration::ZERO).unwrap();
    assert_eq!(
        session.poll_timeout(MESSAGE_TIMEOUT),
        Err(FrameError::TimedOut)
    );
    assert_eq!(session.send(&probe::REQUEST), Err(FrameError::Closed));
}

#[test]
fn arbitrary_payloads_cannot_dispatch_management_or_echo_data() {
    for payload in [
        b"register".as_slice(),
        b"ConnectWifi password",
        b"EnableAccessPoint",
        &probe::RESPONSE,
    ] {
        let mut session = ProbeSession::new();
        assert_eq!(session.respond(payload), Err(FrameError::UnsupportedKind));
        assert_eq!(session.send(&probe::RESPONSE), Err(FrameError::Closed));
    }
}
