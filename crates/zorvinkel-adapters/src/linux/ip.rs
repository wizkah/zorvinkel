//! Device side of the authenticated IP endpoint (ADR-020, ADR-023).
use super::runtime::RuntimeState;
use crate::{
    ble::BleResult,
    ip::{read_record, write_record},
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    time::{Instant, timeout},
};
use zorvinkel_protocol::ip::PORT;

/// Connections beyond this are dropped, so the endpoint cannot exhaust the device.
const MAX_CONNECTIONS: usize = 4;
/// An established session that sends nothing for this long is cut off. The secure session
/// itself ends earlier, by its own lifetime.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// A peer must complete the three handshake records within this time.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const HANDSHAKE_RECORDS: usize = 3;

/// Dual-stack where the system allows it, otherwise IPv4 only.
pub async fn bind() -> std::io::Result<TcpListener> {
    match TcpListener::bind(("::", PORT)).await {
        Ok(listener) => Ok(listener),
        Err(_) => TcpListener::bind(("0.0.0.0", PORT)).await,
    }
}

pub async fn serve(listener: TcpListener, state: Arc<Mutex<RuntimeState>>) -> BleResult<()> {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(_) => {
                // Transient accept failures, such as exhausted descriptors, must not end the service.
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            let _permit = permit;
            // A failed connection only ends that client's session.
            let _ = connection(stream, state).await;
        });
    }
}

async fn connection(mut stream: TcpStream, state: Arc<Mutex<RuntimeState>>) -> BleResult<()> {
    let started = Instant::now();
    let mut responder = {
        let state = state.lock().map_err(|_| "runtime lock failed")?;
        state.new_ip_responder(state.now())?
    };
    let mut records = 0;
    loop {
        let now = state.lock().map_err(|_| "runtime lock failed")?.now();
        if responder.closed(now) {
            break;
        }
        let limit = if records < HANDSHAKE_RECORDS {
            HANDSHAKE_TIMEOUT.saturating_sub(started.elapsed())
        } else {
            IDLE_TIMEOUT
        };
        let record = timeout(limit, read_record(&mut stream))
            .await
            .map_err(|_| "idle connection")??;
        records += 1;
        // Session time is the clock of the timeout poll, not this connection's own start.
        let reply = {
            let mut state = state.lock().map_err(|_| "runtime lock failed")?;
            let now = state.now();
            state.receive_ip(&mut responder, &record, now)?
        };
        if let Some(reply) = reply {
            write_record(&mut stream, &reply).await?;
        }
    }
    Ok(())
}
