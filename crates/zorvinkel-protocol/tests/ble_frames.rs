use std::time::Duration;
use zorvinkel_protocol::ble::*;

#[test]
fn bounded_messages_roundtrip_at_small_and_large_frame_sizes() {
    for frame_length in [20, 64, 512] {
        for length in [1, 8, 9, 255, 256, 4096] {
            let payload: Vec<_> = (0..length).map(|index| index as u8).collect();
            let frames = fragment(1, &payload, frame_length).unwrap();
            let mut receiver = Reassembler::new(frame_length).unwrap();
            for (index, frame) in frames.iter().enumerate() {
                assert!(frame.len() <= frame_length);
                let complete = receiver.receive(frame, Duration::ZERO).unwrap();
                if index + 1 == frames.len() {
                    assert_eq!(complete, Some(payload.clone()));
                } else {
                    assert_eq!(complete, None);
                }
            }
        }
    }
}

#[test]
fn wire_header_has_stable_version_kind_lengths_and_little_endian_fields() {
    let frames = fragment(0x04030201, &[9; 9], 20).unwrap();
    assert_eq!(
        &frames[0][..12],
        &[b'Z', b'V', 1, 0, 1, 2, 3, 4, 9, 0, 0, 0]
    );
    assert_eq!(&frames[1][10..], &[8, 0, 9]);
}

#[test]
fn invalid_sender_limits_are_rejected() {
    for size in [0, 12, 19, 513, usize::MAX] {
        assert!(fragment(1, b"payload", size).is_err());
        assert!(Reassembler::new(size).is_err());
    }
    assert_eq!(
        fragment(0, b"payload", 20),
        Err(FrameError::InvalidMessageId)
    );
    assert_eq!(fragment(1, &[], 20), Err(FrameError::MessageTooLarge));
    assert_eq!(
        fragment(1, &vec![0; MAX_MESSAGE_LENGTH + 1], 20),
        Err(FrameError::MessageTooLarge)
    );
}

#[test]
fn missing_duplicate_and_out_of_order_fragments_close_connection() {
    let frames = fragment(1, &[7; 32], 20).unwrap();
    for index in [0, 2, 3] {
        let mut receiver = Reassembler::new(20).unwrap();
        receiver.receive(&frames[0], Duration::ZERO).unwrap();
        assert_eq!(
            receiver.receive(&frames[index], Duration::ZERO),
            Err(FrameError::UnexpectedFragment)
        );
        assert_eq!(
            receiver.receive(&frames[1], Duration::ZERO),
            Err(FrameError::Closed)
        );
    }
    let mut receiver = Reassembler::new(20).unwrap();
    assert_eq!(
        receiver.receive(&frames[1], Duration::ZERO),
        Err(FrameError::UnexpectedFragment)
    );
}

#[test]
fn timeout_is_absolute_and_releases_partial_message() {
    let frames = fragment(1, &[7; 32], 20).unwrap();
    let mut receiver = Reassembler::new(20).unwrap();
    receiver.receive(&frames[0], Duration::ZERO).unwrap();
    receiver
        .receive(&frames[1], MESSAGE_TIMEOUT - Duration::from_millis(1))
        .unwrap();
    assert_eq!(
        receiver.poll_timeout(MESSAGE_TIMEOUT),
        Err(FrameError::TimedOut)
    );
    assert_eq!(
        receiver.receive(&frames[2], MESSAGE_TIMEOUT),
        Err(FrameError::Closed)
    );
}

#[test]
fn clock_reversal_and_completed_message_replay_are_rejected() {
    let frame = &fragment(1, b"short", 20).unwrap()[0];
    let mut receiver = Reassembler::new(20).unwrap();
    receiver.receive(frame, Duration::from_secs(1)).unwrap();
    assert_eq!(
        receiver.receive(frame, Duration::from_secs(1)),
        Err(FrameError::InvalidMessageId)
    );
    let mut receiver = Reassembler::new(20).unwrap();
    receiver.poll_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        receiver.receive(frame, Duration::from_secs(1)),
        Err(FrameError::ClockMovedBackwards)
    );
}

#[test]
fn malformed_headers_versions_and_overflows_are_rejected_before_buffering() {
    let valid = fragment(1, b"short", 20).unwrap().remove(0);
    for (index, value, expected) in [
        (0, 0, FrameError::Malformed),
        (2, 2, FrameError::UnsupportedVersion),
        (3, 1, FrameError::UnsupportedKind),
        (4, 0, FrameError::InvalidMessageId),
        (9, 255, FrameError::MessageTooLarge),
        (8, 0, FrameError::MessageTooLarge),
        (8, 4, FrameError::UnexpectedFragment),
        (10, 1, FrameError::UnexpectedFragment),
    ] {
        let mut frame = valid.clone();
        frame[index] = value;
        assert_eq!(
            Reassembler::new(20)
                .unwrap()
                .receive(&frame, Duration::ZERO),
            Err(expected)
        );
    }
    for length in 0..=HEADER_LENGTH {
        assert_eq!(
            Reassembler::new(20)
                .unwrap()
                .receive(&valid[..length], Duration::ZERO),
            Err(FrameError::Malformed)
        );
    }
    assert_eq!(
        Reassembler::new(20)
            .unwrap()
            .receive(&[0; 21], Duration::ZERO),
        Err(FrameError::Malformed)
    );
}

#[test]
fn lengths_and_ids_cannot_change_between_fragments() {
    let frames = fragment(1, &[0; 32], 20).unwrap();
    for (index, value, expected) in [
        (8, 33, FrameError::UnexpectedFragment),
        (4, 2, FrameError::InvalidMessageId),
    ] {
        let mut receiver = Reassembler::new(20).unwrap();
        receiver.receive(&frames[0], Duration::ZERO).unwrap();
        let mut changed = frames[1].clone();
        changed[index] = value;
        assert_eq!(receiver.receive(&changed, Duration::ZERO), Err(expected));
    }
}

#[test]
fn consecutive_messages_work_and_explicit_disconnect_discards_state() {
    let mut receiver = Reassembler::new(20).unwrap();
    for id in 1..=3 {
        for frame in fragment(id, b"message", 20).unwrap() {
            assert_eq!(
                receiver.receive(&frame, Duration::ZERO).unwrap(),
                Some(b"message".to_vec())
            );
        }
    }
    receiver.close();
    assert_eq!(
        receiver.poll_timeout(Duration::ZERO),
        Err(FrameError::Closed)
    );
}
