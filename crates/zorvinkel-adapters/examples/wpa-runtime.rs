//! Manual adapter probe. Never selects a network or writes persistent configuration.
#[cfg(target_os = "linux")]
fn main() -> Result<(), &'static str> {
    use std::path::Path;
    use zorvinkel_adapters::linux::network::{WpaNetworkControl, WpaSocket};
    use zorvinkel_application::network::NetworkControl;
    use zorvinkel_core::network::NetworkIntent;
    let probe = match std::env::args().nth(1).as_deref() {
        None => false,
        Some("--probe-profile") => true,
        _ => return Err("usage: wpa-runtime [--probe-profile]"),
    };
    let socket = WpaSocket::open(Path::new("/run/wpa_supplicant/wlan0"), Path::new("/tmp"))
        .map_err(|_| "control socket unavailable")?;
    let mut network = WpaNetworkControl::new(socket);
    let before = network.observe().map_err(|_| "status unavailable")?;
    let count = network
        .cached_scan_results()
        .map_err(|_| "scan cache unavailable")?
        .len();
    println!("Status available; {count} cached network(s)");
    if probe {
        let profile = network
            .prepare_wifi("Zorvinkel-Adapter-Probe", "temporary-test-only")
            .map_err(|_| "temporary profile preparation failed")?;
        let verified = network.check_support(&NetworkIntent::ConnectWifi(profile));
        network
            .discard_profiles()
            .map_err(|_| "temporary profile cleanup failed")?;
        verified.map_err(|_| "temporary profile verification failed")?;
        if network.observe().map_err(|_| "final status unavailable")? != before {
            return Err("observed connection changed during probe");
        }
        println!("Disabled temporary profile verified and removed; connection unchanged");
    }
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This probe requires Linux with wpa_supplicant and dhcpcd.");
}
