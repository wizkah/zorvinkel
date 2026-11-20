#![cfg(feature = "tunnel")]
//! `TunnelOpen` through the secure status session, then the tunnel against a loopback server.
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
};
use zorvinkel_adapters::{
    identity::IdentityHandle,
    memory::{MemoryDeviceTrustStore, MemoryStateStore},
    security::{
        ChannelContext,
        identity::generate_identity,
        service::{ClientSession, DeviceSession, ServiceTime, SessionRegistry},
        simulation::connect,
        tunnel::{PendingTunnel, request},
    },
    tunnel_io::{End, FrameLink, forward},
};
use zorvinkel_application::{
    PortError,
    device::NetworkStatusSource,
    registration::SessionControl,
    state::DeviceState,
    tunnel::{TunnelControl, TunnelRefusal},
};
use zorvinkel_core::{
    NetworkStatus, network::NetworkConfiguration, registry::ClientKey, trust::TrustMode,
};
use zorvinkel_protocol::{
    status::{Operation, RemoteError, ResponseBody},
    tunnel::{CAP_TUNNEL, CloseReason, MAX_FRAME, OpenResponse, WINDOW_FRAMES},
};

const NOW: Duration = Duration::ZERO;

struct Gate(Result<(), TunnelRefusal>);
impl TunnelControl for Gate {
    fn admit(&mut self) -> Result<(), TunnelRefusal> {
        self.0
    }
}
struct Network(Option<Gate>);
impl NetworkStatusSource for Network {
    fn tunnel_control(&mut self) -> Option<&mut dyn TunnelControl> {
        self.0.as_mut().map(|gate| gate as &mut dyn TunnelControl)
    }
    fn read_status(&mut self) -> Result<NetworkStatus, PortError> {
        Ok(NetworkStatus::Disconnected)
    }
}

struct Setup {
    client: ClientSession,
    device: DeviceSession,
    store: MemoryStateStore,
    sessions: SessionRegistry,
    client_key: ClientKey,
}
fn setup(sessions: SessionRegistry) -> Setup {
    let identity = || IdentityHandle::from_material(generate_identity().unwrap());
    let (client_key, device_key) = (identity(), identity());
    let (client, device) = connect(
        &client_key,
        &device_key,
        &mut MemoryDeviceTrustStore::default(),
        TrustMode::TrustOnFirstUse,
        ChannelContext::Simulation,
        NOW,
        &sessions,
    )
    .unwrap();
    let mut setup = Setup {
        client,
        device,
        store: MemoryStateStore::new(DeviceState::initial(NetworkConfiguration::Disconnected)),
        sessions,
        client_key: ClientKey::new(client_key.public_key().to_vec()).unwrap(),
    };
    assert_eq!(
        call(&mut setup, &mut Network(None), Operation::Register, &[]),
        ResponseBody::Registered
    );
    setup
}
fn call(
    setup: &mut Setup,
    network: &mut Network,
    operation: Operation,
    payload: &[u8],
) -> ResponseBody {
    let request = setup
        .client
        .request_payload(operation, payload, NOW)
        .unwrap();
    let time = ServiceTime {
        elapsed: NOW,
        registration_seconds: 0,
    };
    let response = setup
        .device
        .handle(&request, &mut setup.store, network, time)
        .unwrap();
    setup.client.receive(&response, NOW).unwrap().body
}
fn open(
    setup: &mut Setup,
    network: &mut Network,
) -> Result<(PendingTunnel, OpenResponse), RemoteError> {
    let (pending, message) = request(MAX_FRAME as u16).unwrap();
    match call(setup, network, Operation::TunnelOpen, &message.to_bytes()) {
        ResponseBody::TunnelOpened(response) => Ok((pending, response)),
        ResponseBody::Error(error) => Err(error),
        other => panic!("unexpected {other:?}"),
    }
}
fn open_gate() -> Network {
    Network(Some(Gate(Ok(()))))
}

#[test]
fn the_capability_is_announced_only_with_a_tunnel_control() {
    let mut setup = setup(SessionRegistry::default());
    let bits = |network: &mut Network, setup: &mut Setup| match call(
        setup,
        network,
        Operation::Capabilities,
        &[],
    ) {
        ResponseBody::Capabilities(bits) => bits,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(bits(&mut Network(None), &mut setup) & CAP_TUNNEL, 0);
    assert_eq!(bits(&mut open_gate(), &mut setup) & CAP_TUNNEL, CAP_TUNNEL);
}

#[test]
fn a_device_without_tunnel_support_or_with_the_switch_off_refuses() {
    let mut setup = setup(SessionRegistry::default());
    assert_eq!(
        open(&mut setup, &mut Network(None)).err(),
        Some(RemoteError::Unsupported)
    );
    for (refusal, error) in [
        (TunnelRefusal::Unavailable, RemoteError::Unavailable),
        (TunnelRefusal::Busy, RemoteError::Busy),
    ] {
        let mut network = Network(Some(Gate(Err(refusal))));
        assert_eq!(open(&mut setup, &mut network).err(), Some(error));
    }
    assert!(setup.device.take_tunnel().is_none(), "nothing was granted");
    // The refused attempts did not use the slot.
    assert!(open(&mut setup, &mut open_gate()).is_ok());
}

#[test]
fn one_tunnel_per_device_and_the_slot_frees_when_it_ends() {
    let sessions = SessionRegistry::default();
    let mut first = setup(sessions.clone());
    let mut second = setup(sessions.clone());
    assert!(open(&mut first, &mut open_gate()).is_ok());
    assert_eq!(
        open(&mut first, &mut open_gate()).err(),
        Some(RemoteError::Busy),
        "the session already holds one"
    );
    assert_eq!(
        open(&mut second, &mut open_gate()).err(),
        Some(RemoteError::Busy),
        "the device holds one"
    );
    let grant = first.device.take_tunnel().expect("granted");
    drop(grant);
    assert!(open(&mut second, &mut open_gate()).is_ok());
}

#[test]
fn only_the_owner_losing_its_registration_cancels_the_tunnel() {
    let mut setup = setup(SessionRegistry::default());
    open(&mut setup, &mut open_gate()).unwrap();
    let grant = setup.device.take_tunnel().unwrap();
    assert!(!grant.permit.is_cancelled());
    let mut sessions = setup.sessions.clone();
    sessions
        .terminate(&ClientKey::new(vec![9; 32]).unwrap())
        .unwrap();
    assert!(!grant.permit.is_cancelled(), "another client");
    sessions.terminate(&setup.client_key).unwrap();
    assert!(grant.permit.is_cancelled(), "the owner");
}

/// A loopback server standing in for `sshd`: answers in upper case.
async fn upper_case_server() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 512];
        while let Ok(length) = socket.read(&mut buffer).await {
            if length == 0 {
                break;
            }
            buffer[..length].make_ascii_uppercase();
            if socket.write_all(&buffer[..length]).await.is_err() {
                break;
            }
        }
    });
    port
}

#[tokio::test]
async fn bytes_cross_the_tunnel_to_a_loopback_server_and_back() {
    let port = upper_case_server().await;
    let mut setup = setup(SessionRegistry::default());
    let (pending, response) = open(&mut setup, &mut open_gate()).unwrap();
    let grant = setup.device.take_tunnel().unwrap();
    let lifetime = Duration::from_secs(8 * 3600);
    let client_cipher = pending.finish(&response, NOW, lifetime).unwrap();
    let window = response.window;
    assert_eq!(window, WINDOW_FRAMES);

    let (to_device, device_in) = mpsc::channel(64);
    let (to_client, client_in) = mpsc::channel(64);
    let device = tokio::spawn(async move {
        let sshd = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let permit = grant.permit;
        forward(
            grant.cipher,
            window,
            FrameLink {
                incoming: device_in,
                outgoing: to_client,
            },
            sshd,
            || NOW,
            move || !permit.is_cancelled(),
        )
        .await
    });
    let (ssh_end, tunnel_end) = tokio::io::duplex(16 * 1024);
    let client = tokio::spawn(forward(
        client_cipher,
        window,
        FrameLink {
            incoming: client_in,
            outgoing: to_device,
        },
        tunnel_end,
        || NOW,
        || true,
    ));

    // Many frames' worth, so the window and the credit are exercised.
    let message: Vec<u8> = (0..40_000u32)
        .map(|index| b'a' + (index % 26) as u8)
        .collect();
    let (mut read_half, mut write_half) = tokio::io::split(ssh_end);
    let writer_message = message.clone();
    let mut answer = vec![0; message.len()];
    let (written, read) = tokio::join!(
        async move {
            write_half
                .write_all(&writer_message)
                .await
                .map(|()| write_half)
        },
        read_half.read_exact(&mut answer)
    );
    written.unwrap();
    read.unwrap();
    assert_eq!(answer, message.to_ascii_uppercase());

    drop(read_half);
    assert!(matches!(
        client.await.unwrap(),
        End::Stream | End::Peer(CloseReason::Normal)
    ));
    assert!(matches!(
        device.await.unwrap(),
        End::Peer(CloseReason::Normal) | End::Stream
    ));
}
