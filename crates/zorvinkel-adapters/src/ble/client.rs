use super::{BleResult, PROBE, ProbeSession, Profile};
use btleplug::{
    api::{
        Central, CentralEvent, CharPropFlags, Manager as _, Peripheral as _, ScanFilter, WriteType,
    },
    platform::{Adapter, Manager, Peripheral},
};
use futures_util::StreamExt;
use std::{collections::BTreeMap, time::Duration};
use tokio::time::{Instant, timeout, timeout_at};
use uuid::Uuid;
use zorvinkel_core::DiscoveredDevice;
use zorvinkel_protocol::probe;

/// How long after the first hit discovery waits for a missing device name.
const NAME_PATIENCE: Duration = Duration::from_secs(4);

pub struct ProbeCentral {
    adapter: Adapter,
    devices: Vec<Peripheral>,
    profile: Profile,
}
impl ProbeCentral {
    pub async fn open() -> BleResult<Self> {
        Self::open_profile(PROBE).await
    }
    pub async fn open_profile(profile: Profile) -> BleResult<Self> {
        let manager = timeout(Duration::from_secs(10), Manager::new()).await??;
        let adapter = timeout(Duration::from_secs(10), manager.adapters())
            .await??
            .into_iter()
            .next()
            .ok_or("no Bluetooth adapter available")?;
        Ok(Self {
            adapter,
            devices: Vec::new(),
            profile,
        })
    }

    pub async fn discover(&mut self) -> BleResult<Vec<DiscoveredDevice>> {
        self.devices.clear();
        let result = timeout(Duration::from_secs(25), async {
            let mut events = self.adapter.events().await?;
            self.adapter
                .start_scan(ScanFilter {
                    services: vec![Uuid::from_u128(self.profile.service)],
                })
                .await?;
            let maximum_deadline = Instant::now() + Duration::from_secs(20);
            let mut deadline = maximum_deadline;
            let mut observed: BTreeMap<String, (Peripheral, Option<String>)> = BTreeMap::new();
            let mut first_seen = None;
            loop {
                let event = match timeout_at(deadline, events.next()).await {
                    Ok(Some(event)) => event,
                    Ok(None) => return Err("BLE discovery event stream stopped".into()),
                    Err(_) => {
                        // The name may sit in a scan response that follows the first
                        // advertisement (seen on Windows); give it a little longer.
                        let patience = first_seen
                            .map(|seen: Instant| maximum_deadline.min(seen + NAME_PATIENCE));
                        match patience {
                            Some(until)
                                if until > Instant::now()
                                    && observed.values().any(|(_, name)| name.is_none()) =>
                            {
                                deadline = until;
                                continue;
                            }
                            _ => break,
                        }
                    }
                };
                let (id, service_seen) = match event {
                    CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id) => {
                        (id, false)
                    }
                    CentralEvent::ServicesAdvertisement { id, services } => (
                        id,
                        services.contains(&Uuid::from_u128(self.profile.service)),
                    ),
                    _ => continue,
                };
                let peripheral = match self.adapter.peripheral(&id).await {
                    Ok(peripheral) => peripheral,
                    Err(btleplug::Error::DeviceNotFound) => continue,
                    Err(error) => return Err(error.into()),
                };
                let Some(properties) = peripheral.properties().await? else {
                    continue;
                };
                if !service_seen
                    && !properties
                        .services
                        .contains(&Uuid::from_u128(self.profile.service))
                {
                    continue;
                }
                if observed.is_empty() {
                    first_seen = Some(Instant::now());
                    deadline = maximum_deadline.min(Instant::now() + Duration::from_secs(1));
                }
                observed.insert(id.to_string(), (peripheral, properties.local_name));
                if observed.len() == 32 {
                    break;
                }
            }
            let mut discovered = Vec::with_capacity(observed.len());
            for (handle, (peripheral, mut name)) in observed {
                if name.is_none() {
                    name = peripheral
                        .properties()
                        .await
                        .ok()
                        .flatten()
                        .and_then(|properties| properties.local_name);
                }
                discovered.push(DiscoveredDevice {
                    handle,
                    display_name: name.unwrap_or_else(|| "Zorvinkel BLE probe".into()),
                    via_ble: true,
                    via_ip: false,
                });
                self.devices.push(peripheral);
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(discovered)
        })
        .await;
        let stopped = timeout(Duration::from_secs(5), self.adapter.stop_scan()).await;
        if !matches!(&stopped, Ok(Ok(()))) {
            eprintln!("BLE scan cleanup failed");
        }
        let discovered = result??;
        stopped??;
        Ok(discovered)
    }

    /// Each call creates a connection and attempts cleanup even when the probe fails.
    pub async fn probe(&self, handle: &str) -> BleResult<()> {
        let peripheral = self.peripheral(handle).await?;
        let result = timeout(Duration::from_secs(30), exchange(&peripheral)).await;
        let disconnected = timeout(Duration::from_secs(5), peripheral.disconnect()).await;
        if !matches!(&disconnected, Ok(Ok(()))) {
            eprintln!("BLE connection cleanup failed");
        }
        result??;
        disconnected??;
        Ok(())
    }
    pub async fn peripheral(&self, handle: &str) -> BleResult<Peripheral> {
        let peripheral = self
            .devices
            .iter()
            .find(|device| device.id().to_string() == handle)
            .ok_or("device not found in current scan")?;
        // CoreBluetooth releases its native peripheral after disconnect.
        #[cfg(target_os = "macos")]
        let peripheral = timeout(
            Duration::from_secs(5),
            self.adapter
                .retrieve_peripherals(btleplug::api::RetrievePeripheralsOptions {
                    identifiers: Some(vec![peripheral.id()]),
                    services: None,
                }),
        )
        .await??
        .into_iter()
        .find(|device| device.id().to_string() == handle)
        .ok_or("previously discovered device is no longer available")?;
        #[cfg(not(target_os = "macos"))]
        let peripheral = peripheral.clone();
        if timeout(Duration::from_secs(5), peripheral.is_connected()).await?? {
            return Err("device already connected; close its existing connection first".into());
        }
        Ok(peripheral)
    }
}

async fn exchange(peripheral: &Peripheral) -> BleResult<()> {
    peripheral
        .connect()
        .await
        .map_err(|error| format!("BLE probe connection failed: {error}"))?;
    peripheral
        .discover_services()
        .await
        .map_err(|error| format!("BLE probe service discovery failed: {error}"))?;
    let characteristics = peripheral.characteristics();
    let service_uuid = Uuid::from_u128(probe::SERVICE_UUID);
    let write = characteristics
        .iter()
        .find(|c| {
            c.service_uuid == service_uuid
                && c.uuid == Uuid::from_u128(probe::WRITE_UUID)
                && c.properties.contains(CharPropFlags::WRITE)
        })
        .ok_or("probe write characteristic missing")?;
    let notify = characteristics
        .iter()
        .find(|c| {
            c.service_uuid == service_uuid
                && c.uuid == Uuid::from_u128(probe::NOTIFY_UUID)
                && c.properties.contains(CharPropFlags::NOTIFY)
        })
        .ok_or("probe notification characteristic missing")?;
    let mut notifications = peripheral.notifications().await?;
    peripheral
        .subscribe(notify)
        .await
        .map_err(|error| format!("BLE probe notification subscription failed: {error}"))?;
    let mut session = ProbeSession::new();
    let started = Instant::now();
    for frame in session.send(&probe::REQUEST)? {
        peripheral
            .write(write, &frame, WriteType::WithResponse)
            .await
            .map_err(|error| format!("BLE probe fragment write failed: {error}"))?;
    }
    while let Some(notification) = notifications.next().await {
        if notification.uuid != notify.uuid {
            continue;
        }
        if let Some(message) = session.receive(&notification.value, started.elapsed())? {
            if message != probe::RESPONSE {
                return Err("unexpected probe response".into());
            }
            return Ok(());
        }
    }
    Err("notification stream disconnected".into())
}
