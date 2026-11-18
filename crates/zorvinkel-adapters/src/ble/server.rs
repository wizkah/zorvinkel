use super::{BleResult, PROBE, ProbeSession, Profile};
use bluer::{
    Address,
    adv::Advertisement,
    gatt::{
        CharacteristicWriter,
        local::{
            Application, Characteristic, CharacteristicControlEvent, CharacteristicNotify,
            CharacteristicNotifyMethod, CharacteristicWrite, CharacteristicWriteMethod, ReqError,
            Service, characteristic_control,
        },
    },
};
use futures_util::{FutureExt, StreamExt};
use std::{collections::BTreeMap, future::Future, time::Duration};
use tokio::{
    sync::mpsc,
    time::{Instant, timeout},
};
use uuid::Uuid;
use zorvinkel_protocol::{ble::BASELINE_FRAME_LENGTH, probe};

struct Peer {
    writer: CharacteristicWriter,
    session: ProbeSession,
    started: Instant,
    connection: u64,
    closing_at: Option<Instant>,
}

/// Public test service, limited to four notification sessions and 32 queued writes.
/// No application use cases are connected to this endpoint.
pub async fn serve_probe(shutdown: impl Future<Output = ()>) -> BleResult<()> {
    serve(PROBE, "ZV-P3", ProbeBackend, shutdown).await
}

pub trait ServerBackend {
    /// Called once advertising runs; `advertised_name` is what clients see for this device and
    /// `elapsed` the time on the clock that `poll` and `receive` use.
    fn ready(&mut self, _advertised_name: &str, _elapsed: Duration) {}
    fn open(&mut self, _connection: u64, _now: Duration) -> BleResult<()> {
        Ok(())
    }
    fn receive(
        &mut self,
        connection: u64,
        message: &[u8],
        now: Duration,
    ) -> BleResult<Option<Vec<u8>>>;
    fn poll(&mut self, now: Duration) -> BleResult<()>;
    fn closed(&self, connection: u64, now: Duration) -> bool;
    fn remove(&mut self, connection: u64);
}
struct ProbeBackend;
impl ServerBackend for ProbeBackend {
    fn receive(&mut self, _: u64, message: &[u8], _: Duration) -> BleResult<Option<Vec<u8>>> {
        Ok(Some(
            probe::respond(message).ok_or("invalid probe")?.to_vec(),
        ))
    }
    fn poll(&mut self, _: Duration) -> BleResult<()> {
        Ok(())
    }
    fn closed(&self, _: u64, _: Duration) -> bool {
        false
    }
    fn remove(&mut self, _: u64) {}
}

pub async fn serve(
    profile: Profile,
    name: &str,
    mut backend: impl ServerBackend,
    shutdown: impl Future<Output = ()>,
) -> BleResult<()> {
    let started = Instant::now();
    let mut next_connection = 0u64;
    let session = timeout(Duration::from_secs(10), bluer::Session::new())
        .await
        .map_err(|_| "Bluetooth bus connection timed out")??;
    let adapter = timeout(Duration::from_secs(10), session.default_adapter())
        .await
        .map_err(|_| "Bluetooth adapter lookup timed out")??;
    if !timeout(Duration::from_secs(5), adapter.is_powered()).await?? {
        return Err("Bluetooth adapter is powered off".into());
    }
    let (control, notify_handle) = characteristic_control();
    let (writes, mut received) = mpsc::channel::<(Address, Vec<u8>)>(32);
    let application = application(profile, writes, notify_handle);
    let app_handle = timeout(
        Duration::from_secs(10),
        adapter.serve_gatt_application(application),
    )
    .await
    .map_err(|_| "GATT registration timed out")??;
    let address = timeout(Duration::from_secs(5), adapter.address()).await??;
    let suffix = format!("{:02X}{:02X}", address.0[4], address.0[5]);
    let advertisement = Advertisement {
        service_uuids: [Uuid::from_u128(profile.service)].into_iter().collect(),
        discoverable: Some(true),
        discoverable_timeout: Some(Duration::ZERO),
        local_name: Some(format!("{name}-{suffix}")),
        ..Default::default()
    };
    let advertising = timeout(Duration::from_secs(10), adapter.advertise(advertisement))
        .await
        .map_err(|_| "Bluetooth advertising registration timed out")??;
    println!("BLE ready: {name}-{suffix}");
    backend.ready(&format!("{name}-{suffix}"), started.elapsed());
    let mut peers = BTreeMap::<Address, Peer>::new();
    tokio::pin!(shutdown, control);
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let outcome: BleResult<()> = async {
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => break,
                _ = tick.tick() => {
                    backend.poll(started.elapsed())?;
                    let expired: Vec<_> = peers.iter_mut().filter_map(|(address, peer)| {
                        let closed = peer.writer.is_closed().unwrap_or(true);
                        // Let a rejection notification reach the client before server-side disconnect.
                        // A client closing itself avoids racing two native disconnect operations.
                        if backend.closed(peer.connection, started.elapsed()) && peer.closing_at.is_none() { peer.closing_at = Some(Instant::now()); }
                        let closing = peer.closing_at.is_some_and(|at| at.elapsed() >= Duration::from_secs(1));
                        let timed_out = peer.session.poll_timeout(peer.started.elapsed()).is_err() || peer.started.elapsed() >= if profile.service == PROBE.service { Duration::from_secs(60) } else { Duration::from_secs(600) } || closing;
                        (closed || timed_out).then_some((*address, !closed))
                    }).collect();
                    for (address, still_open) in expired {
                        if let Some(peer) = peers.remove(&address) { backend.remove(peer.connection); }
                        // A closed notifier can belong to the previous connection of this peer.
                        if still_open { disconnect(&adapter, address).await; }
                    }
                },
                event = control.next() => match event {
                    Some(CharacteristicControlEvent::Notify(writer)) => {
                        let address = writer.device_address();
                        if peers.contains_key(&address) || peers.len() < 4 {
                            if let Some(old) = peers.remove(&address) { backend.remove(old.connection); }
                            next_connection = next_connection.checked_add(1).ok_or("connection limit")?;
                            backend.open(next_connection, started.elapsed())?;
                            peers.insert(address, Peer { writer, session: ProbeSession::with_kind(profile.kind), started: Instant::now(), connection: next_connection, closing_at: None });
                        } else {
                            disconnect(&adapter, address).await;
                        }
                    }
                    Some(_) => (),
                    None => return Err("notification control stream stopped".into()),
                },
                request = received.recv() => {
                    let Some((address, frame)) = request else { return Err("write queue stopped".into()); };
                    let Some(peer) = peers.get_mut(&address) else { continue; };
                    if let Err(error) = process_frame(peer, &frame, &mut backend, started.elapsed()).await {
                        eprintln!("BLE session failed: {error}");
                        if let Some(peer) = peers.remove(&address) { backend.remove(peer.connection); }
                        disconnect(&adapter, address).await;
                    }
                },

            }
        }
        Ok(())
    }
    .await;
    for (address, peer) in peers {
        backend.remove(peer.connection);
        disconnect(&adapter, address).await;
    }
    let (advertisement, application) = tokio::join!(
        timeout(Duration::from_secs(3), advertising.unregister()),
        timeout(Duration::from_secs(3), app_handle.unregister()),
    );
    outcome?;
    advertisement.map_err(|_| "advertising cleanup timed out")??;
    application.map_err(|_| "GATT cleanup timed out")??;
    Ok(())
}

fn application(
    profile: Profile,
    writes: mpsc::Sender<(Address, Vec<u8>)>,
    notify_handle: bluer::gatt::local::CharacteristicControlHandle,
) -> Application {
    Application {
        services: vec![Service {
            uuid: Uuid::from_u128(profile.service),
            primary: true,
            characteristics: vec![
                Characteristic {
                    uuid: Uuid::from_u128(profile.write),
                    write: Some(CharacteristicWrite {
                        write: true,
                        // BlueZ 5.50 otherwise leaves confirmed writes pending until timeout.
                        write_without_response: true,
                        method: CharacteristicWriteMethod::Fun(Box::new(move |value, request| {
                            let writes = writes.clone();
                            async move {
                                if request.offset != 0 {
                                    return Err(ReqError::InvalidOffset);
                                }
                                if value.len() > BASELINE_FRAME_LENGTH {
                                    return Err(ReqError::InvalidValueLength);
                                }
                                writes
                                    .try_send((request.device_address, value))
                                    .map_err(|_| ReqError::InProgress)
                            }
                            .boxed()
                        })),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                Characteristic {
                    uuid: Uuid::from_u128(profile.notify),
                    notify: Some(CharacteristicNotify {
                        notify: true,
                        method: CharacteristicNotifyMethod::Io,
                        ..Default::default()
                    }),
                    control_handle: notify_handle,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }],
        ..Default::default()
    }
}

async fn process_frame(
    peer: &mut Peer,
    frame: &[u8],
    backend: &mut impl ServerBackend,
    now: Duration,
) -> BleResult<()> {
    if let Some(message) = peer.session.receive(frame, peer.started.elapsed())? {
        let Some(response) = backend.receive(peer.connection, &message, now)? else {
            return Ok(());
        };
        let frames = peer.session.send(&response)?;
        timeout(Duration::from_secs(5), async {
            for frame in frames {
                peer.writer.send(&frame).await?;
            }
            Ok::<_, std::io::Error>(())
        })
        .await??;
    }
    Ok(())
}

async fn disconnect(adapter: &bluer::Adapter, address: Address) {
    let Ok(device) = adapter.device(address) else {
        eprintln!("BLE peer could not be located for cleanup");
        return;
    };
    if !matches!(
        timeout(Duration::from_secs(2), device.disconnect()).await,
        Ok(Ok(()))
    ) {
        eprintln!("BLE peer could not be disconnected");
    }
}
