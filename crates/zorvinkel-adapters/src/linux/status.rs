//! Read-only observation of the existing wpa_supplicant service.
use std::{process::Stdio, time::Duration};
use tokio::{io::AsyncReadExt, process::Command, time::timeout};
use zorvinkel_application::{PortError, device::NetworkStatusSource};
use zorvinkel_core::NetworkStatus;

pub struct ObservedNetwork(pub Result<NetworkStatus, PortError>);
impl NetworkStatusSource for ObservedNetwork {
    fn read_status(&mut self) -> Result<NetworkStatus, PortError> {
        self.0.clone()
    }
}
pub async fn observe(
    interface: &str,
    local_socket_directory: &std::path::Path,
) -> Result<NetworkStatus, PortError> {
    let mut child = Command::new("/sbin/wpa_cli")
        .arg("-s")
        .arg(local_socket_directory)
        .args(["-i", interface, "status"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| PortError::Unavailable)?;
    timeout(Duration::from_secs(2), async {
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .ok_or(PortError::Unavailable)?
            .take(8193)
            .read_to_end(&mut output)
            .await
            .map_err(|_| PortError::Unavailable)?;
        if output.len() > 8192
            || !child
                .wait()
                .await
                .map_err(|_| PortError::Unavailable)?
                .success()
        {
            return Err(PortError::Unavailable);
        }
        parse_status(std::str::from_utf8(&output).map_err(|_| PortError::Unavailable)?)
    })
    .await
    .map_err(|_| PortError::Unavailable)?
}
pub fn parse_status(output: &str) -> Result<NetworkStatus, PortError> {
    let value = |name: &str| output.lines().find_map(|line| line.strip_prefix(name));
    match value("wpa_state=").ok_or(PortError::Unavailable)? {
        "COMPLETED" => {
            let ssid = value("ssid=").ok_or(PortError::Unavailable)?.to_owned();
            // Encoded/non-UTF-8 SSIDs require explicit decoding support.
            if ssid.is_empty()
                || ssid.len() > 32
                || ssid.contains('\\')
                || ssid.chars().any(char::is_control)
            {
                return Err(PortError::Unavailable);
            }
            match value("mode=") {
                Some("AP") => Ok(NetworkStatus::AccessPointActive { ssid }),
                Some("station") | None => Ok(NetworkStatus::WifiConnected { ssid }),
                _ => Err(PortError::Unavailable),
            }
        }
        "DISCONNECTED" | "INACTIVE" | "INTERFACE_DISABLED" | "SCANNING" | "ASSOCIATING"
        | "ASSOCIATED" | "4WAY_HANDSHAKE" | "GROUP_HANDSHAKE" => Ok(NetworkStatus::Disconnected),
        _ => Err(PortError::Unavailable),
    }
}
