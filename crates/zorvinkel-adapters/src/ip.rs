//! Authenticated IP reachability test (ADR-020): record framing on TCP and the client probe.
use crate::ble::BleResult;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zorvinkel_protocol::status::MAX_RECORD;

/// Reads one length-prefixed secure record; the length is validated before any allocation.
pub async fn read_record(reader: &mut (impl AsyncRead + Unpin)) -> BleResult<Vec<u8>> {
    let mut length = [0; 4];
    reader.read_exact(&mut length).await?;
    let payload = u32::from_be_bytes(length) as usize;
    if payload == 0 || payload > MAX_RECORD - 4 {
        return Err("invalid record length".into());
    }
    let mut record = vec![0; 4 + payload];
    record[..4].copy_from_slice(&length);
    reader.read_exact(&mut record[4..]).await?;
    Ok(record)
}
pub async fn write_record(writer: &mut (impl AsyncWrite + Unpin), record: &[u8]) -> BleResult<()> {
    writer.write_all(record).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(feature = "ble-client")]
pub use client::reachability;
#[cfg(feature = "ble-client")]
mod client {
    use super::{read_record, write_record};
    use crate::{
        ble::BleResult,
        identity::IdentityHandle,
        security::{
            ChannelContext,
            handshake::{Handshake, Role},
            service::ClientSession,
        },
    };
    use std::{net::IpAddr, time::Duration};
    use tokio::{
        net::TcpStream,
        time::{Instant, timeout},
    };
    use zorvinkel_core::{IpReachability, trust::DeviceKey};
    use zorvinkel_protocol::status::{Operation, ResponseBody};

    /// Bounds one address, so an unreachable device cannot stall the setup worker for long.
    const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(3);

    /// Succeeds only when the peer proves the expected device identity and accepts this client.
    async fn authenticate(
        address: IpAddr,
        port: u16,
        identity: &IdentityHandle,
        expected_device: &DeviceKey,
    ) -> BleResult<()> {
        let mut stream = TcpStream::connect((address, port)).await?;
        let started = Instant::now();
        let mut handshake =
            Handshake::new(identity, Role::Client, ChannelContext::Ip, Duration::ZERO)?;
        write_record(&mut stream, &handshake.write(started.elapsed())?).await?;
        handshake.read(&read_record(&mut stream).await?, started.elapsed())?;
        // Any host can answer on an address; only the pinned identity counts as the device.
        if handshake.peer_key()?.as_slice() != expected_device.as_bytes() {
            return Err("unexpected device identity".into());
        }
        write_record(&mut stream, &handshake.write(started.elapsed())?).await?;
        let mut session = ClientSession::new(handshake.finish(started.elapsed())?)?;
        let request = session.request(Operation::Authenticate, started.elapsed())?;
        write_record(&mut stream, &request).await?;
        let response = read_record(&mut stream).await?;
        match session.receive(&response, started.elapsed())?.body {
            ResponseBody::Authenticated => Ok(()),
            _ => Err("device rejected this client".into()),
        }
    }

    /// `Authenticated` as soon as one address passes; without addresses nothing was checked.
    pub async fn reachability(
        addresses: &[IpAddr],
        port: u16,
        identity: &IdentityHandle,
        expected_device: &DeviceKey,
    ) -> IpReachability {
        if addresses.is_empty() {
            return IpReachability::Unverified;
        }
        let attempts = addresses.iter().map(|address| async move {
            timeout(
                ATTEMPT_TIMEOUT,
                authenticate(*address, port, identity, expected_device),
            )
            .await
            .is_ok_and(|result| result.is_ok())
        });
        if futures_util::future::join_all(attempts)
            .await
            .into_iter()
            .any(|reached| reached)
        {
            IpReachability::Authenticated
        } else {
            IpReachability::Unreachable
        }
    }
}
