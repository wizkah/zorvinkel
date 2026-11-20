//! Radio measurement service: throughput, latency and negotiated sizes of a BLE link.
//!
//! It exists to put numbers behind the limits of the terminal tunnel (ADR-031). It has its own
//! service, carries no application data and is started by hand (`--ble-bench`).
use super::Profile;
use std::time::Duration;
use zorvinkel_protocol::ble::MessageKind;

pub const BENCH: Profile = Profile {
    kind: MessageKind::Probe,
    service: 0x719e40a0_4a4b_4d75_9c08_a9cbb88f4300,
    write: 0x719e40a0_4a4b_4d75_9c08_a9cbb88f4301,
    notify: 0x719e40a0_4a4b_4d75_9c08_a9cbb88f4302,
};

/// Smallest frame the measurements use; the header of the largest message needs 7 bytes.
pub const MIN_FRAME: usize = 8;
const ECHO: u8 = 1;
const SINK: u8 = 2;
const REPORT: u8 = 3;
const SOURCE: u8 = 4;
const SOURCED: u8 = 5;
const INFO: u8 = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    /// The device answers with the same bytes.
    Echo { sequence: u32 },
    /// The device only counts the frame.
    Sink,
    /// The device answers with what it counted since the last report and starts again.
    Report,
    /// The device sends `count` notifications of `size` bytes each.
    Source { size: u16, count: u32 },
    /// The device answers with the size of notifications it can send.
    Info,
}
impl Command {
    /// A frame of exactly `size` bytes (padded with a fixed pattern), at least [`MIN_FRAME`].
    pub fn frame(self, size: usize) -> Vec<u8> {
        let mut frame = vec![0xA5; size.max(MIN_FRAME)];
        match self {
            Self::Echo { sequence } => {
                frame[0] = ECHO;
                frame[1..5].copy_from_slice(&sequence.to_le_bytes());
            }
            Self::Sink => frame[0] = SINK,
            Self::Report => frame[0] = REPORT,
            Self::Source { size, count } => {
                frame[0] = SOURCE;
                frame[1..3].copy_from_slice(&size.to_le_bytes());
                frame[3..7].copy_from_slice(&count.to_le_bytes());
            }
            Self::Info => frame[0] = INFO,
        }
        frame
    }
    pub fn parse(frame: &[u8]) -> Option<Self> {
        if frame.len() < MIN_FRAME {
            return None;
        }
        Some(match frame[0] {
            ECHO => Self::Echo {
                sequence: u32::from_le_bytes(frame[1..5].try_into().ok()?),
            },
            SINK => Self::Sink,
            REPORT => Self::Report,
            SOURCE => Self::Source {
                size: u16::from_le_bytes(frame[1..3].try_into().ok()?),
                count: u32::from_le_bytes(frame[3..7].try_into().ok()?),
            },
            INFO => Self::Info,
            _ => return None,
        })
    }
}

/// What the device counted between two reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counted {
    pub frames: u32,
    pub bytes: u32,
    /// From the first counted frame to the last one.
    pub elapsed: Duration,
}
impl Counted {
    pub fn to_frame(self) -> Vec<u8> {
        let mut frame = vec![REPORT];
        frame.extend(self.frames.to_le_bytes());
        frame.extend(self.bytes.to_le_bytes());
        frame.extend(
            u64::try_from(self.elapsed.as_micros())
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        frame
    }
    pub fn parse(frame: &[u8]) -> Option<Self> {
        if frame.len() != 17 || frame[0] != REPORT {
            return None;
        }
        Some(Self {
            frames: u32::from_le_bytes(frame[1..5].try_into().ok()?),
            bytes: u32::from_le_bytes(frame[5..9].try_into().ok()?),
            elapsed: Duration::from_micros(u64::from_le_bytes(frame[9..17].try_into().ok()?)),
        })
    }
}

pub fn info_frame(notify_size: u16) -> Vec<u8> {
    let mut frame = vec![INFO];
    frame.extend(notify_size.to_le_bytes());
    frame
}
pub fn parse_info(frame: &[u8]) -> Option<u16> {
    (frame.len() == 3 && frame[0] == INFO).then(|| u16::from_le_bytes([frame[1], frame[2]]))
}
/// A notification of a `Source` run: the first bytes carry the number of the frame.
pub fn sourced_frame(sequence: u32, size: usize) -> Vec<u8> {
    let mut frame = vec![0x5A; size.max(MIN_FRAME)];
    frame[0] = SOURCED;
    frame[1..5].copy_from_slice(&sequence.to_le_bytes());
    frame
}
pub fn is_sourced(frame: &[u8]) -> bool {
    frame.len() >= MIN_FRAME && frame[0] == SOURCED
}

/// Bytes per second, `None` when no time passed.
pub fn rate(bytes: u64, elapsed: Duration) -> Option<f64> {
    (!elapsed.is_zero()).then(|| bytes as f64 / elapsed.as_secs_f64())
}

/// Ordered samples with the percentiles that matter for interactive use.
pub struct Samples(Vec<Duration>);
impl Samples {
    pub fn new(mut samples: Vec<Duration>) -> Self {
        samples.sort();
        Self(samples)
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// Nearest-rank percentile; `None` without samples.
    pub fn percentile(&self, percent: usize) -> Option<Duration> {
        if self.0.is_empty() || percent > 100 {
            return None;
        }
        let rank = (percent * self.0.len()).div_ceil(100).max(1);
        self.0.get(rank - 1).copied()
    }
}

#[cfg(all(feature = "ble-server", target_os = "linux"))]
pub use server::serve_bench;
#[cfg(all(feature = "ble-server", target_os = "linux"))]
mod server {
    use super::*;
    use crate::ble::BleResult;
    use bluer::{
        Address,
        adv::Advertisement,
        gatt::{
            CharacteristicWriter,
            local::{
                Application, Characteristic, CharacteristicControlEvent, CharacteristicNotify,
                CharacteristicNotifyMethod, CharacteristicWrite, CharacteristicWriteMethod,
                Service, characteristic_control,
            },
        },
    };
    use futures_util::{FutureExt, StreamExt};
    use std::future::Future;
    use tokio::{
        sync::mpsc,
        time::{Instant, timeout},
    };
    use uuid::Uuid;

    /// One peer at a time; frames of up to 512 bytes are accepted.
    pub async fn serve_bench(shutdown: impl Future<Output = ()>) -> BleResult<()> {
        let session = timeout(Duration::from_secs(10), bluer::Session::new()).await??;
        let adapter = timeout(Duration::from_secs(10), session.default_adapter()).await??;
        if !timeout(Duration::from_secs(5), adapter.is_powered()).await?? {
            return Err("Bluetooth adapter is powered off".into());
        }
        let (control, notify_handle) = characteristic_control();
        let (writes, mut received) = mpsc::channel::<(Address, Vec<u8>)>(256);
        let application = Application {
            services: vec![Service {
                uuid: Uuid::from_u128(BENCH.service),
                primary: true,
                characteristics: vec![
                    Characteristic {
                        uuid: Uuid::from_u128(BENCH.write),
                        write: Some(CharacteristicWrite {
                            write: true,
                            write_without_response: true,
                            method: CharacteristicWriteMethod::Fun(Box::new(
                                move |value, request| {
                                    let writes = writes.clone();
                                    async move {
                                        // A full queue slows the sender down instead of losing frames.
                                        writes
                                            .send((request.device_address, value))
                                            .await
                                            .map_err(|_| bluer::gatt::local::ReqError::Failed)
                                    }
                                    .boxed()
                                },
                            )),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    Characteristic {
                        uuid: Uuid::from_u128(BENCH.notify),
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
        };
        let app_handle = adapter.serve_gatt_application(application).await?;
        let address = adapter.address().await?;
        let name = format!("ZV-BENCH-{:02X}{:02X}", address.0[4], address.0[5]);
        let advertising = adapter
            .advertise(Advertisement {
                service_uuids: [Uuid::from_u128(BENCH.service)].into_iter().collect(),
                discoverable: Some(true),
                discoverable_timeout: Some(Duration::ZERO),
                local_name: Some(name.clone()),
                ..Default::default()
            })
            .await?;
        println!("BLE bench ready: {name}");
        let mut writer: Option<std::sync::Arc<CharacteristicWriter>> = None;
        let mut source: Option<tokio::task::JoinHandle<()>> = None;
        let mut counter = Counter::default();
        tokio::pin!(shutdown, control);
        let outcome: BleResult<()> = async {
            loop {
                tokio::select! {
                    () = &mut shutdown => break,
                    event = control.next() => match event {
                        Some(CharacteristicControlEvent::Notify(new)) => {
                            println!("peer {} subscribed, notification size {}", new.device_address(), new.mtu());
                            if let Some(task) = source.take() {
                                task.abort();
                            }
                            writer = Some(std::sync::Arc::new(new));
                            counter = Counter::default();
                        }
                        Some(_) => {}
                        None => return Err("notification control stream stopped".into()),
                    },
                    request = received.recv() => {
                        let Some((_, frame)) = request else { return Err("write queue stopped".into()); };
                        handle(&frame, writer.as_ref(), &mut counter, &mut source).await?;
                    }
                }
            }
            Ok(())
        }
        .await;
        let _ = timeout(Duration::from_secs(3), advertising.unregister()).await;
        let _ = timeout(Duration::from_secs(3), app_handle.unregister()).await;
        outcome
    }

    #[derive(Default)]
    struct Counter {
        first: Option<Instant>,
        last: Option<Instant>,
        frames: u32,
        bytes: u32,
    }
    impl Counter {
        fn count(&mut self, length: usize) {
            let now = Instant::now();
            self.first.get_or_insert(now);
            self.last = Some(now);
            self.frames += 1;
            self.bytes = self
                .bytes
                .saturating_add(u32::try_from(length).unwrap_or(u32::MAX));
        }
        fn take(&mut self) -> Counted {
            let counted = Counted {
                frames: self.frames,
                bytes: self.bytes,
                elapsed: match (self.first, self.last) {
                    (Some(first), Some(last)) => last - first,
                    _ => Duration::ZERO,
                },
            };
            *self = Self::default();
            counted
        }
    }

    async fn handle(
        frame: &[u8],
        writer: Option<&std::sync::Arc<CharacteristicWriter>>,
        counter: &mut Counter,
        source: &mut Option<tokio::task::JoinHandle<()>>,
    ) -> BleResult<()> {
        let Some(command) = Command::parse(frame) else {
            eprintln!("unknown frame of {} bytes", frame.len());
            return Ok(());
        };
        if !matches!(command, Command::Sink | Command::Echo { .. }) {
            eprintln!("command {command:?} ({} bytes)", frame.len());
        }
        // A new command ends a run that is still sending, so that it cannot block the answer.
        if !matches!(command, Command::Sink | Command::Echo { .. })
            && let Some(task) = source.take()
        {
            task.abort();
        }
        match command {
            Command::Sink => counter.count(frame.len()),
            Command::Echo { .. } => {
                if let Some(writer) = writer {
                    writer.send(frame).await?;
                }
            }
            Command::Report => {
                if let Some(writer) = writer {
                    writer.send(&counter.take().to_frame()).await?;
                }
            }
            Command::Info => {
                if let Some(writer) = writer {
                    let size = u16::try_from(writer.mtu()).unwrap_or(u16::MAX);
                    writer.send(&info_frame(size)).await?;
                }
            }
            Command::Source { size, count } => {
                if let Some(writer) = writer {
                    let writer = std::sync::Arc::clone(writer);
                    *source = Some(tokio::spawn(async move {
                        for sequence in 0..count {
                            let frame = sourced_frame(sequence, usize::from(size));
                            if writer.send(&frame).await.is_err() {
                                break;
                            }
                        }
                    }));
                }
            }
        }
        Ok(())
    }
}

#[cfg(feature = "ble-client")]
pub use client::run_bench;
#[cfg(feature = "ble-client")]
mod client {
    use super::*;
    use crate::ble::{BleResult, client::ProbeCentral};
    use btleplug::{
        api::{CharPropFlags, Characteristic, Peripheral as _, WriteType},
        platform::Peripheral,
    };
    use futures_util::{Stream, StreamExt};
    use std::pin::Pin;
    use tokio::time::{Instant, timeout};
    use uuid::Uuid;

    type Notifications = Pin<Box<dyn Stream<Item = btleplug::api::ValueNotification> + Send>>;
    const SIZES: [usize; 12] = [20, 64, 128, 185, 244, 256, 300, 400, 500, 509, 511, 512];
    const BURSTS: [u32; 6] = [8, 16, 32, 64, 128, 256];
    /// Data volume of the sustained runs, whatever the frame size.
    const TOTAL_BYTES: usize = 200_000;
    fn frames_for(size: usize) -> u32 {
        u32::try_from(TOTAL_BYTES / size).unwrap_or(u32::MAX)
    }

    struct Link {
        peripheral: Peripheral,
        write: Characteristic,
        notifications: Notifications,
        notify_uuid: Uuid,
    }
    impl Link {
        async fn open(peripheral: &Peripheral) -> BleResult<Self> {
            if !peripheral.is_connected().await? {
                timeout(Duration::from_secs(20), peripheral.connect()).await??;
            }
            peripheral.discover_services().await?;
            let service = Uuid::from_u128(BENCH.service);
            let find = |uuid: u128, property: CharPropFlags| {
                peripheral
                    .characteristics()
                    .into_iter()
                    .find(|c| {
                        c.service_uuid == service
                            && c.uuid == Uuid::from_u128(uuid)
                            && c.properties.contains(property)
                    })
                    .ok_or("bench characteristic missing")
            };
            let write = find(BENCH.write, CharPropFlags::WRITE_WITHOUT_RESPONSE)?;
            let notify = find(BENCH.notify, CharPropFlags::NOTIFY)?;
            let notifications = peripheral.notifications().await?;
            peripheral.subscribe(&notify).await?;
            Ok(Self {
                peripheral: peripheral.clone(),
                write,
                notifications,
                notify_uuid: notify.uuid,
            })
        }
        async fn send(&self, frame: &[u8], kind: WriteType) -> BleResult<()> {
            timeout(
                Duration::from_secs(10),
                self.peripheral.write(&self.write, frame, kind),
            )
            .await??;
            Ok(())
        }
        async fn next(&mut self, within: Duration) -> BleResult<Vec<u8>> {
            loop {
                let notification = timeout(within, self.notifications.next())
                    .await
                    .map_err(|_| "no notification in time")?
                    .ok_or("notification stream ended")?;
                if notification.uuid == self.notify_uuid {
                    return Ok(notification.value);
                }
            }
        }
        /// Ends a run the device may still be sending and waits until its last frame passed:
        /// notifications arrive in order, so the answer to the info request comes last.
        async fn drain(&mut self) -> BleResult<()> {
            self.send(&Command::Info.frame(MIN_FRAME), WriteType::WithoutResponse)
                .await?;
            loop {
                let frame = self.next(Duration::from_secs(90)).await?;
                if parse_info(&frame).is_some() {
                    return Ok(());
                }
            }
        }
        /// Releases the connection so that a stuck write queue starts afresh.
        async fn reconnect(self) -> BleResult<Self> {
            let peripheral = self.peripheral.clone();
            drop(self);
            let _ = timeout(Duration::from_secs(5), peripheral.disconnect()).await;
            tokio::time::sleep(Duration::from_secs(2)).await;
            Self::open(&peripheral).await
        }
    }

    /// Finds the bench service, measures it and prints a table. `handle` skips the question
    /// which device to use when only one is found.
    pub async fn run_bench(handle: Option<&str>) -> BleResult<()> {
        let mut central = ProbeCentral::open_profile(BENCH).await?;
        let devices = central.discover().await?;
        for device in &devices {
            println!("found {}: {}", device.handle, device.display_name);
        }
        let chosen = match (handle, devices.as_slice()) {
            (Some(handle), _) => handle.to_owned(),
            (None, [only]) => only.handle.clone(),
            _ => return Err("pass the handle of one bench device".into()),
        };
        let peripheral = central.peripheral(&chosen).await?;
        let result = measure(&peripheral).await;
        let _ = timeout(Duration::from_secs(5), peripheral.disconnect()).await;
        result
    }

    async fn measure(peripheral: &Peripheral) -> BleResult<()> {
        let mut link = Link::open(peripheral).await?;
        link.send(&Command::Info.frame(MIN_FRAME), WriteType::WithoutResponse)
            .await?;
        let device_notify = usize::from(
            parse_info(&link.next(Duration::from_secs(5)).await?)
                .ok_or("unexpected info answer")?,
        );
        let client_mtu = usize::from(peripheral.mtu());
        println!("client ATT MTU {client_mtu}, device notification size {device_notify}");

        println!("\nlargest write the device receives (3 echoes each):");
        let mut writable = Vec::new();
        for size in SIZES
            .into_iter()
            .filter(|size| *size <= client_mtu.saturating_sub(3))
        {
            match echoes(&mut link, size, WriteType::WithoutResponse, 3).await {
                Ok(_) => {
                    println!("  {size:>4} bytes  ok");
                    writable.push(size);
                }
                Err(error) => {
                    println!("  {size:>4} bytes  not delivered ({error})");
                    // A write that never arrives can wedge the queue; start on a fresh link.
                    link = link.reconnect().await?;
                    break;
                }
            }
        }
        let max_write = writable.last().copied().unwrap_or(0);

        println!("\nround trip (write, device echoes, notification):");
        for (size, kind, label) in [
            (20, WriteType::WithoutResponse, "without response"),
            (20, WriteType::WithResponse, "with response"),
            (max_write, WriteType::WithoutResponse, "without response"),
        ] {
            if size < MIN_FRAME {
                continue;
            }
            match echoes(&mut link, size, kind, 100).await {
                Ok(samples) => println!(
                    "  {size:>4} bytes {label:<18} median {:>6.1} ms  p95 {:>6.1} ms  max {:>6.1} ms",
                    millis(samples.percentile(50)),
                    millis(samples.percentile(95)),
                    millis(samples.percentile(100)),
                ),
                Err(error) => println!("  {size:>4} bytes {label:<18} failed: {error}"),
            }
        }

        println!("\ndevice to client, bursts without flow control (frames lost per burst size):");
        let mut header = String::from("       size ");
        for burst in BURSTS {
            header.push_str(&format!(" {burst:>7}"));
        }
        println!("{header}");
        for size in SIZES.into_iter().filter(|size| *size <= device_notify) {
            let mut line = format!("  {size:>4} bytes ");
            for burst in BURSTS {
                let (received, _) =
                    download(&mut link, size, burst, Duration::from_secs(4)).await?;
                line.push_str(&format!(" {:>7}", burst - received.min(burst)));
            }
            println!("{line}");
        }

        println!("\nround trip while the device streams 185-byte frames without flow control:");
        match echoes_under_load(&mut link, 185, 1100).await {
            Ok(samples) => println!(
                "  median {:>6.0} ms  max {:>6.0} ms  ({} echoes)",
                millis(samples.percentile(50)),
                millis(samples.percentile(100)),
                samples.len()
            ),
            Err(error) => println!("  failed: {error}"),
        }

        println!("\ndevice to client, sustained (200000 bytes each, at most 60 s):");
        for size in SIZES.into_iter().filter(|size| *size <= device_notify) {
            let count = frames_for(size);
            let (received, elapsed) =
                download(&mut link, size, count, Duration::from_secs(60)).await?;
            println!(
                "  {size:>4} bytes  lost {:>4} of {count}  {:>7.1} kB/s",
                count - received.min(count),
                rate(u64::from(received) * size as u64, elapsed).unwrap_or(0.0) / 1000.0
            );
        }

        println!(
            "\nclient to device, sustained (write without response, {TOTAL_BYTES} bytes each):"
        );
        for size in writable.iter().copied() {
            match upload(&mut link, size, frames_for(size)).await {
                Ok((sent, counted)) => println!(
                    "  {size:>4} bytes  sent {sent:>5}  lost {:>4}  {:>7.1} kB/s",
                    sent.saturating_sub(counted.frames),
                    rate(u64::from(counted.bytes), counted.elapsed).unwrap_or(0.0) / 1000.0
                ),
                Err(error) => println!("  {size:>4} bytes  failed: {error}"),
            }
        }
        Ok(())
    }

    fn millis(duration: Option<Duration>) -> f64 {
        duration.map_or(f64::NAN, |value| value.as_secs_f64() * 1000.0)
    }

    /// How long a small request waits while a bulk stream is queued on the device: BlueZ queues
    /// notifications without bound, so this is the delay a window-less tunnel would add.
    async fn echoes_under_load(link: &mut Link, size: usize, frames: u32) -> BleResult<Samples> {
        link.drain().await?;
        link.send(
            &Command::Source {
                size: u16::try_from(size)?,
                count: frames,
            }
            .frame(MIN_FRAME),
            WriteType::WithoutResponse,
        )
        .await?;
        let mut samples = Vec::new();
        for sequence in 0..6 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let started = Instant::now();
            link.send(
                &Command::Echo { sequence }.frame(20),
                WriteType::WithoutResponse,
            )
            .await?;
            loop {
                let frame = link.next(Duration::from_secs(60)).await?;
                if Command::parse(&frame) == Some(Command::Echo { sequence }) {
                    samples.push(started.elapsed());
                    break;
                }
            }
        }
        link.drain().await?;
        Ok(Samples::new(samples))
    }

    async fn echoes(
        link: &mut Link,
        size: usize,
        kind: WriteType,
        count: u32,
    ) -> BleResult<Samples> {
        let mut samples = Vec::new();
        for sequence in 0..count {
            let started = Instant::now();
            link.send(&Command::Echo { sequence }.frame(size), kind)
                .await?;
            let echoed = link.next(Duration::from_secs(3)).await?;
            if Command::parse(&echoed) != Some(Command::Echo { sequence }) {
                return Err("unexpected echo".into());
            }
            samples.push(started.elapsed());
        }
        Ok(Samples::new(samples))
    }

    async fn upload(link: &mut Link, size: usize, frames: u32) -> BleResult<(u32, Counted)> {
        // Clear the device's counters, then send as fast as the stack accepts.
        link.drain().await?;
        link.send(
            &Command::Report.frame(MIN_FRAME),
            WriteType::WithoutResponse,
        )
        .await?;
        link.next(Duration::from_secs(5)).await?;
        let frame = Command::Sink.frame(size);
        let mut sent = 0u32;
        for _ in 0..frames {
            link.send(&frame, WriteType::WithoutResponse).await?;
            sent += 1;
        }
        // Writes arrive in order, so the report is answered after the last frame that arrives.
        link.send(
            &Command::Report.frame(MIN_FRAME),
            WriteType::WithoutResponse,
        )
        .await?;
        let report = link.next(Duration::from_secs(90)).await?;
        let counted = Counted::parse(&report).ok_or("unexpected report")?;
        Ok((sent, counted))
    }

    /// Frames received of `count` the device sends at once, and the time from first to last.
    async fn download(
        link: &mut Link,
        size: usize,
        count: u32,
        within: Duration,
    ) -> BleResult<(u32, Duration)> {
        link.drain().await?;
        let size16 = u16::try_from(size)?;
        link.send(
            &Command::Source {
                size: size16,
                count,
            }
            .frame(MIN_FRAME),
            WriteType::WithoutResponse,
        )
        .await?;
        let mut received = 0u32;
        let mut first = None;
        let mut last = Instant::now();
        let deadline = Instant::now() + within;
        while received < count {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let Ok(frame) = link.next(remaining.max(Duration::from_millis(1))).await else {
                break;
            };
            if is_sourced(&frame) {
                first.get_or_insert_with(Instant::now);
                last = Instant::now();
                received += 1;
            }
        }
        let elapsed = first.map_or(Duration::ZERO, |first| last - first);
        Ok((received, elapsed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_survive_a_round_trip_at_every_size() {
        for size in [8, 20, 244, 512] {
            for command in [
                Command::Echo { sequence: 7 },
                Command::Sink,
                Command::Report,
                Command::Source {
                    size: 185,
                    count: 2000,
                },
                Command::Info,
            ] {
                let frame = command.frame(size);
                assert_eq!(frame.len(), size);
                assert_eq!(Command::parse(&frame), Some(command));
            }
        }
    }
    #[test]
    fn frames_are_never_shorter_than_the_header() {
        assert_eq!(Command::Sink.frame(1).len(), MIN_FRAME);
        assert_eq!(Command::parse(&[ECHO, 0, 0]), None);
        assert_eq!(Command::parse(&[0xFF; 20]), None);
    }
    #[test]
    fn reports_and_info_round_trip() {
        let counted = Counted {
            frames: 1200,
            bytes: 222_000,
            elapsed: Duration::from_millis(3500),
        };
        assert_eq!(Counted::parse(&counted.to_frame()), Some(counted));
        assert_eq!(Counted::parse(&[REPORT, 1, 2]), None);
        assert_eq!(parse_info(&info_frame(244)), Some(244));
        assert_eq!(parse_info(&[INFO]), None);
    }
    #[test]
    fn sourced_frames_are_recognised_by_their_marker() {
        assert!(is_sourced(&sourced_frame(3, 20)));
        assert!(!is_sourced(&Command::Sink.frame(20)));
    }
    #[test]
    fn rate_needs_elapsed_time() {
        assert_eq!(rate(1000, Duration::ZERO), None);
        assert_eq!(rate(1000, Duration::from_millis(500)), Some(2000.0));
    }
    #[test]
    fn percentiles_use_the_nearest_rank() {
        let samples = Samples::new((1..=100).rev().map(Duration::from_millis).collect());
        assert_eq!(samples.percentile(50), Some(Duration::from_millis(50)));
        assert_eq!(samples.percentile(95), Some(Duration::from_millis(95)));
        assert_eq!(samples.percentile(100), Some(Duration::from_millis(100)));
        assert_eq!(Samples::new(Vec::new()).percentile(50), None);
        assert_eq!(samples.percentile(101), None);
    }
}
