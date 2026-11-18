use std::{
    fs::{self, DirBuilder},
    io,
    os::unix::{fs::DirBuilderExt, net::UnixDatagram},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use zorvinkel_core::network::NetworkError;

/// Separate sockets prevent late replies from being mistaken for a subsequent reply.
pub struct WpaSocket {
    remote: PathBuf,
    directory: PathBuf,
}
static NEXT_SOCKET: AtomicU64 = AtomicU64::new(0);

impl WpaSocket {
    /// `local_directory` must be writable by the caller and reachable by wpa_supplicant.
    pub fn open(remote: &Path, local_directory: &Path) -> Result<Self, NetworkError> {
        let directory = local_directory.join(format!(
            "wpa-{}-{}",
            std::process::id(),
            NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)
        ));
        DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(map_io)?;
        Ok(Self {
            remote: remote.into(),
            directory,
        })
    }
}
impl super::WpaControl for WpaSocket {
    fn scan_complete(&mut self) -> Result<(), NetworkError> {
        let path = self
            .directory
            .join(format!("s{}", NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)));
        let result = (|| {
            let socket = UnixDatagram::bind(&path).map_err(map_io)?;
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .map_err(map_io)?;
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .map_err(map_io)?;
            socket.connect(&self.remote).map_err(map_io)?;
            socket.send(b"ATTACH").map_err(map_io)?;
            let mut bytes = [0; 4096];
            let length = socket.recv(&mut bytes).map_err(map_io)?;
            if bytes[..length].trim_ascii() != b"OK" {
                return Err(NetworkError::Rejected);
            }
            let result = (|| {
                if super::WpaControl::request(self, "SCAN")?.trim() != "OK" {
                    return Err(NetworkError::Rejected);
                }
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                loop {
                    let remaining = deadline
                        .checked_duration_since(std::time::Instant::now())
                        .ok_or(NetworkError::TimedOut)?;
                    socket.set_read_timeout(Some(remaining)).map_err(map_io)?;
                    let length = socket.recv(&mut bytes).map_err(map_io)?;
                    let event = std::str::from_utf8(&bytes[..length])
                        .map_err(|_| NetworkError::Unavailable)?;
                    let kind = event
                        .strip_prefix('<')
                        .and_then(|event| event.split_once('>'))
                        .and_then(|(_, event)| event.split_whitespace().next());
                    if kind == Some("CTRL-EVENT-SCAN-FAILED") {
                        return Err(NetworkError::Rejected);
                    }
                    if kind == Some("CTRL-EVENT-SCAN-RESULTS") {
                        return Ok(());
                    }
                }
            })();
            let _ = socket.send(b"DETACH");
            result
        })();
        let _ = fs::remove_file(path);
        result
    }
    fn request(&mut self, command: &str) -> Result<String, NetworkError> {
        let path = self
            .directory
            .join(format!("c{}", NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)));
        let result = (|| {
            let socket = UnixDatagram::bind(&path).map_err(map_io)?;
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .map_err(map_io)?;
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .map_err(map_io)?;
            socket.connect(&self.remote).map_err(map_io)?;
            socket.send(command.as_bytes()).map_err(map_io)?;
            let mut buffer = vec![0; 65_537];
            let length = socket.recv(&mut buffer).map_err(map_io)?;
            if length > 65_536 {
                return Err(NetworkError::Unavailable);
            }
            String::from_utf8(buffer[..length].to_vec()).map_err(|_| NetworkError::Unavailable)
        })();
        let _ = fs::remove_file(path);
        result
    }
}
impl Drop for WpaSocket {
    fn drop(&mut self) {
        let _ = fs::remove_dir(&self.directory);
    }
}
fn map_io(error: io::Error) -> NetworkError {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => NetworkError::TimedOut,
        _ => NetworkError::Unavailable,
    }
}
