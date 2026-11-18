#![cfg(all(feature = "security", feature = "ble-client"))]
use std::{
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::net::{TcpListener, TcpStream};
use zorvinkel_adapters::{
    ble::secure_client::StatusConnection,
    identity::IdentityHandle,
    ip::{reachability, read_record, write_record},
    memory::{MemoryDeviceTrustStore, MemoryStateStore},
    security::{
        ChannelContext,
        ble::Responder,
        handshake::{Handshake, Role},
        identity::generate_identity,
        service::{ClientSession, ServiceTime, SessionRegistry},
    },
    simulation::SimulatedNetwork,
};
use zorvinkel_application::{registration::register, state::DeviceState};
use zorvinkel_core::{
    IpReachability, network::NetworkConfiguration, registry::ClientKey, trust::DeviceKey,
};
use zorvinkel_protocol::status::{Operation, RemoteError, ResponseBody};

const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

fn identity() -> IdentityHandle {
    IdentityHandle::from_material(generate_identity().unwrap())
}
fn device_key(identity: &IdentityHandle) -> DeviceKey {
    DeviceKey::new(identity.public_key().to_vec()).unwrap()
}

/// Serves the endpoint like the device service does, for the clients registered in the store.
async fn serve(device: Arc<IdentityHandle>, registered: &IdentityHandle) -> u16 {
    let listener = TcpListener::bind((LOOPBACK, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut store = MemoryStateStore::new(DeviceState::initial(NetworkConfiguration::Disconnected));
    register(
        &mut store,
        ClientKey::new(registered.public_key().to_vec()).unwrap(),
        0,
    )
    .unwrap();
    let sessions = SessionRegistry::default();
    tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let started = Instant::now();
            let mut responder = Responder::new_ip(&device, started.elapsed()).unwrap();
            while !responder.closed(started.elapsed()) {
                let Ok(record) = read_record(&mut stream).await else {
                    break;
                };
                let time = ServiceTime {
                    elapsed: started.elapsed(),
                    registration_seconds: 0,
                };
                let reply = responder.receive(
                    &record,
                    &mut store,
                    &mut SimulatedNetwork::default(),
                    time,
                    &sessions,
                );
                match reply {
                    Ok(Some(reply)) => write_record(&mut stream, &reply).await.unwrap(),
                    Ok(None) => (),
                    Err(_) => break,
                }
            }
        }
    });
    port
}

#[tokio::test]
async fn a_registered_client_confirms_the_device_over_ip() {
    let (client, device) = (identity(), Arc::new(identity()));
    let port = serve(device.clone(), &client).await;
    let result = reachability(&[LOOPBACK], port, &client, &device_key(&device)).await;
    assert_eq!(result, IpReachability::Authenticated);
}

#[tokio::test]
async fn one_working_address_among_unreachable_ones_is_enough() {
    let (client, device) = (identity(), Arc::new(identity()));
    let port = serve(device.clone(), &client).await;
    let unreachable = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));
    let result = reachability(
        &[unreachable, LOOPBACK],
        port,
        &client,
        &device_key(&device),
    )
    .await;
    assert_eq!(result, IpReachability::Authenticated);
}

#[tokio::test]
async fn an_unregistered_client_is_not_confirmed() {
    let (registered, stranger, device) = (identity(), identity(), Arc::new(identity()));
    let port = serve(device.clone(), &registered).await;
    let result = reachability(&[LOOPBACK], port, &stranger, &device_key(&device)).await;
    assert_eq!(result, IpReachability::Unreachable);
}

#[tokio::test]
async fn another_device_on_the_address_is_not_confirmed() {
    let (client, impostor) = (identity(), Arc::new(identity()));
    let port = serve(impostor, &client).await;
    let expected = device_key(&identity());
    let result = reachability(&[LOOPBACK], port, &client, &expected).await;
    assert_eq!(result, IpReachability::Unreachable);
}

#[tokio::test]
async fn a_closed_port_is_unreachable_and_no_address_is_unverified() {
    let (client, device) = (identity(), identity());
    let port = {
        let listener = TcpListener::bind((LOOPBACK, 0)).await.unwrap();
        listener.local_addr().unwrap().port()
    };
    let key = device_key(&device);
    assert_eq!(
        reachability(&[LOOPBACK], port, &client, &key).await,
        IpReachability::Unreachable
    );
    assert_eq!(
        reachability(&[], port, &client, &key).await,
        IpReachability::Unverified
    );
}

#[tokio::test]
async fn the_endpoint_serves_the_same_operations_as_bluetooth() {
    let (client, device) = (identity(), Arc::new(identity()));
    let port = serve(device.clone(), &client).await;
    let started = Instant::now();
    let mut stream = TcpStream::connect((LOOPBACK, port)).await.unwrap();
    let mut handshake =
        Handshake::new(&client, Role::Client, ChannelContext::Ip, Duration::ZERO).unwrap();
    write_record(&mut stream, &handshake.write(started.elapsed()).unwrap())
        .await
        .unwrap();
    let second = read_record(&mut stream).await.unwrap();
    handshake.read(&second, started.elapsed()).unwrap();
    write_record(&mut stream, &handshake.write(started.elapsed()).unwrap())
        .await
        .unwrap();
    let mut session = ClientSession::new(handshake.finish(started.elapsed()).unwrap()).unwrap();
    let mut ask = async |operation: Operation| {
        let request = session.request(operation, started.elapsed()).unwrap();
        write_record(&mut stream, &request).await.unwrap();
        let response = read_record(&mut stream).await.unwrap();
        session.receive(&response, started.elapsed()).unwrap().body
    };
    assert_eq!(ask(Operation::Register).await, ResponseBody::Registered);
    assert_eq!(
        ask(Operation::Authenticate).await,
        ResponseBody::Authenticated
    );
    assert!(matches!(
        ask(Operation::Status).await,
        ResponseBody::Status { mode: 0, .. }
    ));
    assert!(matches!(
        ask(Operation::Capabilities).await,
        ResponseBody::Capabilities(_)
    ));
    // Operations the device does not offer stay refused, as over Bluetooth.
    assert_eq!(
        ask(Operation::SystemInfo).await,
        ResponseBody::Error(RemoteError::Unsupported)
    );
}

#[tokio::test]
async fn the_client_connection_pins_the_device_and_carries_requests() {
    let (client, device) = (identity(), Arc::new(identity()));
    let port = serve(device.clone(), &client).await;
    let mut trust = MemoryDeviceTrustStore::default();
    let mut connection = StatusConnection::connect_ip(&[LOOPBACK], port, &client, &mut trust)
        .await
        .unwrap();
    assert!(connection.is_ip());
    assert_eq!(
        connection.request(Operation::Authenticate).await.unwrap(),
        ResponseBody::Authenticated
    );
    connection.close().await;
    // The pinned identity now rejects a different device on the same address.
    let impostor = Arc::new(identity());
    let port = serve(impostor, &client).await;
    assert!(
        StatusConnection::connect_ip(&[LOOPBACK], port, &client, &mut trust)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn oversized_records_are_refused_before_allocation() {
    let (mut near, mut far) = tokio::io::duplex(64);
    tokio::io::AsyncWriteExt::write_all(&mut far, &u32::MAX.to_be_bytes())
        .await
        .unwrap();
    assert!(read_record(&mut near).await.is_err());
}
