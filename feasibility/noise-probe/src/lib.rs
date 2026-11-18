//! Hardwarefreier Versuch; kein produktives Sicherheitsprofil.

#[cfg(test)]
mod tests {
    use snow::{Builder, HandshakeState, Keypair, TransportState};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    const PROFILE: &str = "Noise_XX_25519_ChaChaPoly_SHA256";
    const CONTEXT: &[u8] = b"zorvinkel-feasibility-v1";
    const MAX_FRAME: usize = 4096;

    fn keypair() -> Keypair {
        Builder::new(PROFILE.parse().unwrap())
            .generate_keypair()
            .unwrap()
    }

    fn handshake(key: &Keypair, initiator: bool) -> HandshakeState {
        let builder = Builder::new(PROFILE.parse().unwrap())
            .local_private_key(&key.private)
            .unwrap()
            .prologue(CONTEXT)
            .unwrap();
        if initiator {
            builder.build_initiator().unwrap()
        } else {
            builder.build_responder().unwrap()
        }
    }

    fn message(sender: &mut HandshakeState) -> Vec<u8> {
        let mut frame = vec![0; MAX_FRAME];
        let length = sender.write_message(&[], &mut frame).unwrap();
        frame.truncate(length);
        frame
    }

    fn receive(receiver: &mut HandshakeState, frame: &[u8]) {
        receiver.read_message(frame, &mut [0; MAX_FRAME]).unwrap();
    }

    fn session(client: &Keypair, server: &Keypair) -> (TransportState, TransportState) {
        let mut c = handshake(client, true);
        let mut s = handshake(server, false);
        receive(&mut s, &message(&mut c));
        receive(&mut c, &message(&mut s));
        receive(&mut s, &message(&mut c));
        assert_eq!(c.get_remote_static().unwrap(), server.public);
        assert_eq!(s.get_remote_static().unwrap(), client.public);
        (
            c.into_transport_mode().unwrap(),
            s.into_transport_mode().unwrap(),
        )
    }

    fn encrypt(sender: &mut TransportState, data: &[u8]) -> Vec<u8> {
        let mut frame = vec![0; MAX_FRAME];
        let length = sender.write_message(data, &mut frame).unwrap();
        frame.truncate(length);
        frame
    }

    #[test]
    fn xx_exchanges_and_proves_both_static_keys() {
        let (mut client, mut server) = session(&keypair(), &keypair());
        let frame = encrypt(&mut client, b"network-status");
        let mut plain = [0; MAX_FRAME];
        let length = server.read_message(&frame, &mut plain).unwrap();
        assert_eq!(&plain[..length], b"network-status");
        let reply = encrypt(&mut server, b"offline");
        let length = client.read_message(&reply, &mut plain).unwrap();
        assert_eq!(&plain[..length], b"offline");
    }

    #[test]
    fn changed_pinned_server_key_is_detectable_after_handshake() {
        let expected = keypair();
        let replacement = keypair();
        let mut client = handshake(&keypair(), true);
        let mut server = handshake(&replacement, false);
        receive(&mut server, &message(&mut client));
        receive(&mut client, &message(&mut server));
        // XX allein vertraut der Gegenstelle nicht: Die Anwendung muss hier abbrechen.
        assert_ne!(client.get_remote_static().unwrap(), expected.public);
    }

    #[test]
    fn altered_ciphertext_is_rejected() {
        let (mut client, mut server) = session(&keypair(), &keypair());
        let mut frame = encrypt(&mut client, b"network-status");
        frame[0] ^= 1;
        assert!(server.read_message(&frame, &mut [0; MAX_FRAME]).is_err());
    }

    #[test]
    fn replay_in_same_session_is_rejected() {
        let (mut client, mut server) = session(&keypair(), &keypair());
        let frame = encrypt(&mut client, b"network-status");
        server.read_message(&frame, &mut [0; MAX_FRAME]).unwrap();
        assert!(server.read_message(&frame, &mut [0; MAX_FRAME]).is_err());
    }

    #[test]
    fn old_ciphertext_is_rejected_in_new_session_with_same_identities() {
        let client_key = keypair();
        let server_key = keypair();
        let (mut old_client, _) = session(&client_key, &server_key);
        let old_frame = encrypt(&mut old_client, b"network-status");
        let (_, mut new_server) = session(&client_key, &server_key);
        assert!(
            new_server
                .read_message(&old_frame, &mut [0; MAX_FRAME])
                .is_err()
        );
    }

    #[test]
    fn xx_handshake_message_sizes_are_measured() {
        let mut client = handshake(&keypair(), true);
        let mut server = handshake(&keypair(), false);
        let mut sizes = Vec::new();
        for initiator_turn in [true, false, true] {
            let (sender, receiver) = if initiator_turn {
                (&mut client, &mut server)
            } else {
                (&mut server, &mut client)
            };
            let frame = message(sender);
            sizes.push(frame.len());
            receive(receiver, &frame);
        }
        assert!(client.is_handshake_finished());
        assert!(server.is_handshake_finished());
        println!("XX-Handshake ohne Nutzdaten: {sizes:?} Byte");
    }

    fn send(stream: &mut TcpStream, frame: &[u8]) {
        assert!(frame.len() <= MAX_FRAME);
        stream
            .write_all(&(frame.len() as u32).to_be_bytes())
            .unwrap();
        stream.write_all(frame).unwrap();
    }

    fn read_frame(stream: &mut TcpStream) -> Vec<u8> {
        let mut size = [0; 4];
        stream.read_exact(&mut size).unwrap();
        let size = u32::from_be_bytes(size) as usize;
        assert!(size <= MAX_FRAME);
        let mut frame = vec![0; size];
        stream.read_exact(&mut frame).unwrap();
        frame
    }

    fn timeouts(stream: &TcpStream) {
        let timeout = Some(Duration::from_secs(5));
        stream.set_read_timeout(timeout).unwrap();
        stream.set_write_timeout(timeout).unwrap();
    }

    #[test]
    fn xx_and_encrypted_request_reply_work_over_loopback_tcp() {
        let server_key = keypair();
        let expected_server = server_key.public.clone();
        let client_key = keypair();
        let expected_client = client_key.public.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            timeouts(&socket);
            let mut server = handshake(&server_key, false);
            receive(&mut server, &read_frame(&mut socket));
            send(&mut socket, &message(&mut server));
            receive(&mut server, &read_frame(&mut socket));
            assert_eq!(server.get_remote_static().unwrap(), expected_client);
            let mut server = server.into_transport_mode().unwrap();
            let request = read_frame(&mut socket);
            let mut plain = [0; MAX_FRAME];
            let length = server.read_message(&request, &mut plain).unwrap();
            assert_eq!(&plain[..length], b"network-status");
            send(&mut socket, &encrypt(&mut server, b"offline"));
        });
        let mut socket = TcpStream::connect_timeout(&address, Duration::from_secs(5)).unwrap();
        timeouts(&socket);
        let mut client = handshake(&client_key, true);
        send(&mut socket, &message(&mut client));
        receive(&mut client, &read_frame(&mut socket));
        assert_eq!(client.get_remote_static().unwrap(), expected_server);
        send(&mut socket, &message(&mut client));
        let mut client = client.into_transport_mode().unwrap();
        send(&mut socket, &encrypt(&mut client, b"network-status"));
        let reply = read_frame(&mut socket);
        let mut plain = [0; MAX_FRAME];
        let length = client.read_message(&reply, &mut plain).unwrap();
        assert_eq!(&plain[..length], b"offline");
        worker.join().unwrap();
    }
}
