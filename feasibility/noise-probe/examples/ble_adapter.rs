//! Nur Adapterstatus; kein Scan, keine Verbindung und keine Systemänderung.

use btleplug::api::{Central, Manager as _};
use btleplug::platform::Manager;
use std::error::Error;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tokio::time::timeout(Duration::from_secs(10), inspect()).await?
}

async fn inspect() -> Result<(), Box<dyn Error>> {
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    println!("Sichtbare Bluetooth-Adapter: {}", adapters.len());
    for adapter in adapters {
        println!("{:?}", adapter.adapter_state().await?);
    }
    Ok(())
}
