use std::{fs, path::PathBuf};
use zorvinkel_adapters::persistence::FileStateStore;
use zorvinkel_application::state::DeviceState;
use zorvinkel_core::network::NetworkConfiguration;
pub struct Fixture(pub PathBuf);
impl Fixture {
    pub fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "zorvinkel-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    pub fn directory(&self) -> PathBuf {
        self.0.join("device")
    }
    pub fn initialize(&self) -> FileStateStore {
        FileStateStore::initialize(
            &self.directory(),
            DeviceState::initial(NetworkConfiguration::Disconnected),
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
