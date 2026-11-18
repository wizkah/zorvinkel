use super::{BleResult, ProbeSession, STATUS};
use crate::{
    identity::IdentityHandle,
    ip::{read_record, write_record},
    security::{
        ChannelContext,
        handshake::{Handshake, Role},
        service::ClientSession,
    },
};
use btleplug::{
    api::{CharPropFlags, Characteristic, Peripheral as _, ValueNotification, WriteType},
    platform::Peripheral,
};
use futures_util::{Stream, StreamExt};
use std::{net::IpAddr, pin::Pin, time::Duration};
use tokio::{
    io::AsyncWriteExt,
    net::TcpStream,
    time::{Instant, timeout},
};
use uuid::Uuid;
use zorvinkel_application::trust::{DeviceTrustStore, check_device, verify_device};
use zorvinkel_core::trust::{DeviceKey, TrustMode};
use zorvinkel_protocol::status::{Operation, ResponseBody};

type Notifications = Pin<Box<dyn Stream<Item = ValueNotification> + Send>>;
/// Bluetooth and IP carry the same secured records; only the framing underneath differs.
enum Link {
    Ble {
        peripheral: Peripheral,
        write: Characteristic,
        notify: Uuid,
        notifications: Notifications,
        frames: Box<ProbeSession>,
    },
    Ip(TcpStream),
}
pub struct StatusConnection {
    link: Link,
    started: Instant,
    session: Option<ClientSession>,
}
/// Bounds one address, so an unreachable one cannot stall the connection for long.
const IP_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Records kept free for one refresh and the renewal that follows it.
const RECORD_RESERVE: usize = 64;
impl StatusConnection {
    /// Connects over IP to the first of `addresses` that accepts. The device is trusted only by
    /// its proven identity, never by the address or announcement that led here.
    pub async fn connect_ip(
        addresses: &[IpAddr],
        port: u16,
        identity: &IdentityHandle,
        trust: &mut impl DeviceTrustStore,
    ) -> BleResult<Self> {
        if addresses.is_empty() {
            return Err("no address to connect to".into());
        }
        let attempts = addresses.iter().map(|address| {
            Box::pin(async move {
                timeout(IP_CONNECT_TIMEOUT, TcpStream::connect((*address, port)))
                    .await
                    .map_err(|_| "IP connection timed out")?
                    .map_err(|error| format!("IP connection failed: {error}"))
            })
        });
        let (stream, _) = futures_util::future::select_ok(attempts).await?;
        let mut connection = Self {
            link: Link::Ip(stream),
            started: Instant::now(),
            session: None,
        };
        connection
            .establish(identity, trust, ChannelContext::Ip)
            .await?;
        Ok(connection)
    }

    /// The caller disconnects the peripheral on any failed or cancelled connection attempt.
    pub async fn connect(
        peripheral: Peripheral,
        identity: &IdentityHandle,
        trust: &mut impl DeviceTrustStore,
    ) -> BleResult<Self> {
        timeout(Duration::from_secs(5), peripheral.connect())
            .await
            .map_err(|_| "BLE connection timed out")?
            .map_err(|error| format!("BLE connection failed: {error}"))?;
        timeout(Duration::from_secs(15), peripheral.discover_services())
            .await
            .map_err(|_| "BLE service discovery timed out")?
            .map_err(|error| format!("BLE service discovery failed: {error}"))?;
        let characteristics = peripheral.characteristics();
        let find = |uuid, flag| {
            characteristics
                .iter()
                .find(|c| {
                    c.service_uuid == Uuid::from_u128(STATUS.service)
                        && c.uuid == Uuid::from_u128(uuid)
                        && c.properties.contains(flag)
                })
                .cloned()
        };
        let write = find(STATUS.write, CharPropFlags::WRITE)
            .ok_or("status write characteristic missing")?;
        let notify = find(STATUS.notify, CharPropFlags::NOTIFY)
            .ok_or("status notification characteristic missing")?;
        let notifications = peripheral.notifications().await?;
        timeout(Duration::from_secs(5), peripheral.subscribe(&notify))
            .await
            .map_err(|_| "BLE subscription timed out")?
            .map_err(|error| format!("BLE subscription failed: {error}"))?;
        let mut connection = Self {
            link: Link::Ble {
                peripheral,
                write,
                notify: notify.uuid,
                notifications,
                frames: Box::new(ProbeSession::with_kind(STATUS.kind)),
            },
            started: Instant::now(),
            session: None,
        };
        connection
            .establish(identity, trust, ChannelContext::Ble)
            .await?;
        Ok(connection)
    }
    /// Noise handshake with trust-on-first-use pinning of the device key.
    async fn establish(
        &mut self,
        identity: &IdentityHandle,
        trust: &mut impl DeviceTrustStore,
        context: ChannelContext,
    ) -> BleResult<()> {
        let mut handshake = Handshake::new(identity, Role::Client, context, Duration::ZERO)?;
        let first = handshake.write(self.started.elapsed())?;
        self.send(&first).await?;
        let second = self.receive().await?;
        handshake.read(&second, self.started.elapsed())?;
        let peer =
            DeviceKey::new(handshake.peer_key()?.to_vec()).map_err(|_| "invalid device key")?;
        check_device(trust, peer.clone(), TrustMode::TrustOnFirstUse)
            .map_err(|_| crate::security::SecureError::Trust)?;
        let third = handshake.write(self.started.elapsed())?;
        self.send(&third).await?;
        self.session = Some(ClientSession::new(
            handshake.finish(self.started.elapsed())?,
        )?);
        verify_device(trust, peer, TrustMode::TrustOnFirstUse)
            .map_err(|_| crate::security::SecureError::Trust)?;
        Ok(())
    }
    /// Time left before the secure profile ends the session; zero when the record budget is
    /// nearly used up too, because the session must then be renewed just the same.
    pub fn session_remaining(&self) -> Option<Duration> {
        let (time, records) = self.session.as_ref()?.remaining(self.started.elapsed());
        Some(if records < RECORD_RESERVE {
            Duration::ZERO
        } else {
            time
        })
    }
    /// Whether this connection runs over the network instead of Bluetooth.
    pub fn is_ip(&self) -> bool {
        matches!(self.link, Link::Ip(_))
    }
    async fn send(&mut self, record: &[u8]) -> BleResult<()> {
        match &mut self.link {
            Link::Ble {
                peripheral,
                write,
                frames,
                ..
            } => {
                let frames = frames.send(record)?;
                timeout(Duration::from_secs(5), async {
                    for frame in frames {
                        peripheral
                            .write(write, &frame, WriteType::WithResponse)
                            .await?;
                    }
                    Ok::<_, btleplug::Error>(())
                })
                .await??;
            }
            Link::Ip(stream) => {
                timeout(Duration::from_secs(5), write_record(stream, record)).await??;
            }
        }
        Ok(())
    }
    async fn receive(&mut self) -> BleResult<Vec<u8>> {
        match &mut self.link {
            Link::Ble {
                notify,
                notifications,
                frames,
                ..
            } => {
                let started = self.started;
                timeout(Duration::from_secs(5), async {
                    while let Some(notification) = notifications.next().await {
                        if notification.uuid == *notify
                            && let Some(record) =
                                frames.receive(&notification.value, started.elapsed())?
                        {
                            return Ok(record);
                        }
                    }
                    Err("BLE notification stream disconnected".into())
                })
                .await?
            }
            Link::Ip(stream) => timeout(Duration::from_secs(5), read_record(stream)).await?,
        }
    }
    pub async fn request(&mut self, operation: Operation) -> BleResult<ResponseBody> {
        self.request_payload(operation, &[]).await
    }
    pub async fn request_payload(
        &mut self,
        operation: Operation,
        payload: &[u8],
    ) -> BleResult<ResponseBody> {
        let result = self.request_inner(operation, payload).await;
        if result.is_err() {
            self.close().await;
        }
        result
    }
    async fn request_inner(
        &mut self,
        operation: Operation,
        payload: &[u8],
    ) -> BleResult<ResponseBody> {
        let record = self
            .session
            .as_mut()
            .ok_or("session closed")?
            .request_payload(operation, payload, self.started.elapsed())?;
        self.send(&record).await?;
        let record = self.receive().await?;
        Ok(self
            .session
            .as_mut()
            .ok_or("session closed")?
            .receive(&record, self.started.elapsed())?
            .body)
    }
    pub async fn close(&mut self) {
        if let Some(mut session) = self.session.take() {
            session.close();
        }
        match &mut self.link {
            Link::Ble { peripheral, .. } => {
                let _ = timeout(Duration::from_secs(3), peripheral.disconnect()).await;
            }
            Link::Ip(stream) => {
                let _ = timeout(Duration::from_secs(3), stream.shutdown()).await;
            }
        }
    }
}
