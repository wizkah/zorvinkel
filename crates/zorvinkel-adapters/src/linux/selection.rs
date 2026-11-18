//! Startup selection of the network stack (ADR-015). Probing is read-only and never
//! changes network state; an unclear situation yields a limitation instead of a switch.
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkStack {
    NetworkManager,
    WpaDhcpcd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limitation {
    /// NetworkManager is responsible, but this build has no NetworkManager adapter.
    NetworkManagerUnsupported,
    /// No wpa_supplicant control socket for the interface.
    WpaSupplicantMissing,
    /// wpa_supplicant runs without dhcpcd; IP configuration would be unmanaged.
    DhcpcdMissing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    Selected(NetworkStack),
    Limited(Limitation),
}

pub trait StackProbe {
    fn network_manager_running(&self) -> bool;
    fn wpa_supplicant_control(&self, interface: &str) -> bool;
    fn dhcpcd_running(&self) -> bool;
}

/// NetworkManager has priority; wpa_supplicant/dhcpcd is only a fallback when
/// NetworkManager is absent and both services are present for the interface.
pub fn select(probe: &impl StackProbe, interface: &str) -> Selection {
    if probe.network_manager_running() {
        return Selection::Selected(NetworkStack::NetworkManager);
    }
    if !probe.wpa_supplicant_control(interface) {
        return Selection::Limited(Limitation::WpaSupplicantMissing);
    }
    if !probe.dhcpcd_running() {
        return Selection::Limited(Limitation::DhcpcdMissing);
    }
    Selection::Selected(NetworkStack::WpaDhcpcd)
}

pub struct SystemProbe {
    run: PathBuf,
    proc: PathBuf,
}
impl Default for SystemProbe {
    fn default() -> Self {
        Self::new("/run")
    }
}
impl SystemProbe {
    pub fn new(run: impl Into<PathBuf>) -> Self {
        Self::with_proc(run, "/proc")
    }
    pub fn with_proc(run: impl Into<PathBuf>, proc: impl Into<PathBuf>) -> Self {
        Self {
            run: run.into(),
            proc: proc.into(),
        }
    }
    /// Debian starts NetworkManager under systemd with `--no-daemon`, which writes no PID file.
    fn process_named(&self, name: &str) -> bool {
        let Ok(entries) = std::fs::read_dir(&self.proc) else {
            return false;
        };
        entries.flatten().any(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
                && std::fs::read_to_string(entry.path().join("comm"))
                    .is_ok_and(|comm| comm.trim_end() == name)
        })
    }
    fn running(&self, pid_file: &Path) -> bool {
        let Ok(pid) = std::fs::read_to_string(self.run.join(pid_file)) else {
            return false;
        };
        pid.trim()
            .parse::<u32>()
            .is_ok_and(|pid| pid > 0 && Path::new("/proc").join(pid.to_string()).exists())
    }
}
impl StackProbe for SystemProbe {
    fn network_manager_running(&self) -> bool {
        self.running(Path::new("NetworkManager/NetworkManager.pid"))
            || self.process_named("NetworkManager")
    }
    fn wpa_supplicant_control(&self, interface: &str) -> bool {
        self.run.join("wpa_supplicant").join(interface).exists()
    }
    fn dhcpcd_running(&self) -> bool {
        self.running(Path::new("dhcpcd.pid")) || self.running(Path::new("dhcpcd/pid"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Probe(bool, bool, bool);
    impl StackProbe for Probe {
        fn network_manager_running(&self) -> bool {
            self.0
        }
        fn wpa_supplicant_control(&self, _: &str) -> bool {
            self.1
        }
        fn dhcpcd_running(&self) -> bool {
            self.2
        }
    }
    #[test]
    fn network_manager_has_priority_and_is_never_bypassed() {
        for probe in [Probe(true, true, true), Probe(true, false, false)] {
            assert_eq!(
                select(&probe, "wlan0"),
                Selection::Selected(NetworkStack::NetworkManager)
            );
        }
    }
    #[test]
    fn fallback_requires_wpa_supplicant_and_dhcpcd() {
        assert_eq!(
            select(&Probe(false, true, true), "wlan0"),
            Selection::Selected(NetworkStack::WpaDhcpcd)
        );
        assert_eq!(
            select(&Probe(false, false, true), "wlan0"),
            Selection::Limited(Limitation::WpaSupplicantMissing)
        );
        assert_eq!(
            select(&Probe(false, true, false), "wlan0"),
            Selection::Limited(Limitation::DhcpcdMissing)
        );
    }
    #[test]
    fn system_probe_requires_a_live_process() {
        let root = std::env::temp_dir().join(format!("zv-select-{}", std::process::id()));
        std::fs::create_dir_all(root.join("wpa_supplicant")).unwrap();
        std::fs::write(root.join("dhcpcd.pid"), std::process::id().to_string()).unwrap();
        std::fs::write(root.join("wpa_supplicant/wlan0"), "").unwrap();
        let probe = SystemProbe::new(&root);
        assert!(probe.dhcpcd_running() == Path::new("/proc/self").exists());
        assert!(probe.wpa_supplicant_control("wlan0"));
        assert!(!probe.wpa_supplicant_control("wlan1"));
        let proc = root.join("proc");
        std::fs::create_dir_all(proc.join("42")).unwrap();
        std::fs::write(proc.join("42/comm"), "NetworkManager\n").unwrap();
        std::fs::create_dir_all(proc.join("self")).unwrap();
        std::fs::write(proc.join("self/comm"), "NetworkManager\n").unwrap();
        let probe = SystemProbe::with_proc(&root, &proc);
        assert!(probe.network_manager_running());
        std::fs::write(proc.join("42/comm"), "NetworkManager-dispatcher\n").unwrap();
        std::fs::remove_dir_all(proc.join("self")).unwrap();
        assert!(!probe.network_manager_running());
        std::fs::create_dir_all(root.join("NetworkManager")).unwrap();
        std::fs::write(
            root.join("NetworkManager/NetworkManager.pid"),
            std::process::id().to_string(),
        )
        .unwrap();
        assert!(probe.network_manager_running() == Path::new("/proc/self").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
