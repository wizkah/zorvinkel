//! Shutdown and restart for the unprivileged service. It only creates a request file in its
//! runtime directory; root-owned systemd path units (see `packaging/linux`) perform the action.
use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
use zorvinkel_application::power::{PowerAction, PowerControl, PowerError};

pub const UNIT_DIRECTORY: &str = "/etc/systemd/system";
const PATH_UNITS: [&str; 2] = [
    "zorvinkel-power@poweroff.path",
    "zorvinkel-power@reboot.path",
];
/// Lets the encrypted acknowledgement leave the device before services stop.
const ACKNOWLEDGEMENT_DELAY: Duration = Duration::from_secs(2);

/// The feature is only offered where the installer enabled both path units.
pub fn installed(unit_directory: &Path) -> bool {
    PATH_UNITS.iter().all(|unit| {
        unit_directory
            .join("multi-user.target.wants")
            .join(unit)
            .exists()
    })
}
fn request_name(action: PowerAction) -> &'static str {
    match action {
        PowerAction::Shutdown => "poweroff",
        PowerAction::Restart => "reboot",
    }
}

pub struct SystemPower {
    directory: PathBuf,
    delay: Duration,
    scheduled: Arc<AtomicBool>,
}
impl SystemPower {
    pub fn new(directory: &Path) -> Self {
        Self::with_delay(directory, ACKNOWLEDGEMENT_DELAY)
    }
    fn with_delay(directory: &Path, delay: Duration) -> Self {
        Self {
            directory: directory.into(),
            delay,
            scheduled: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl PowerControl for SystemPower {
    fn schedule(&mut self, action: PowerAction) -> Result<(), PowerError> {
        if self.scheduled.swap(true, Ordering::SeqCst) {
            return Err(PowerError::Busy);
        }
        let request = self.directory.join(request_name(action));
        let (delay, scheduled) = (self.delay, self.scheduled.clone());
        let spawned = thread::Builder::new()
            .name("power-request".into())
            .spawn(move || {
                thread::sleep(delay);
                if let Err(error) = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&request)
                {
                    eprintln!("Power request could not be written: {error}");
                    // A failed request must not block a later retry.
                    scheduled.store(false, Ordering::SeqCst);
                }
            });
        if spawned.is_err() {
            self.scheduled.store(false, Ordering::SeqCst);
            return Err(PowerError::Unavailable);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn directory(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("zorvinkel-power-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }
    fn wait_for(path: &Path) -> bool {
        (0..200).any(|_| {
            thread::sleep(Duration::from_millis(10));
            path.exists()
        })
    }
    #[test]
    fn each_action_creates_its_own_request_file() {
        for (action, name) in [
            (PowerAction::Shutdown, "poweroff"),
            (PowerAction::Restart, "reboot"),
        ] {
            let dir = directory(name);
            let mut power = SystemPower::with_delay(&dir, Duration::ZERO);
            power.schedule(action).unwrap();
            assert!(wait_for(&dir.join(name)));
            std::fs::remove_dir_all(dir).unwrap();
        }
    }
    #[test]
    fn a_scheduled_action_blocks_further_requests() {
        let dir = directory("busy");
        let mut power = SystemPower::with_delay(&dir, Duration::from_secs(60));
        power.schedule(PowerAction::Restart).unwrap();
        assert_eq!(power.schedule(PowerAction::Shutdown), Err(PowerError::Busy));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn a_failed_request_can_be_retried() {
        let dir = directory("retry");
        let mut power = SystemPower::with_delay(&dir.join("missing"), Duration::ZERO);
        power.schedule(PowerAction::Shutdown).unwrap();
        assert!((0..200).any(|_| {
            thread::sleep(Duration::from_millis(10));
            !power.scheduled.load(Ordering::SeqCst)
        }));
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn units_must_be_enabled_for_the_feature_to_be_offered() {
        let dir = directory("units");
        assert!(!installed(&dir));
        let wants = dir.join("multi-user.target.wants");
        std::fs::create_dir_all(&wants).unwrap();
        for unit in PATH_UNITS {
            std::fs::write(wants.join(unit), b"").unwrap();
        }
        assert!(installed(&dir));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
