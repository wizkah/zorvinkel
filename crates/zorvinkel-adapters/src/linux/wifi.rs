//! Bounded setup worker; BLE dispatch only queues work and reads task snapshots.
use super::{
    access_point::{self, AccessPointLink},
    network::{WpaNetworkControl, WpaSocket},
    stack::{StackControl, profile_ssid},
};
use crate::persistence::FileStateStore;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};
use zeroize::Zeroizing;
use zorvinkel_application::{
    network::{execute, reconcile, set_restart_policy},
    setup::AccessPointSettings,
    state::StateStore,
    wifi::{RestartChange, RestartState, WifiError, WifiSetupControl, WifiTaskState},
};
use zorvinkel_core::{network::*, registry::ClientKey};

/// Opt-in flag, default off: its existence means successful connections are saved by
/// wpa_supplicant. Changed only through local administration, never through BLE.
pub fn persistence_path(root: &Path) -> PathBuf {
    root.join("wifi-persistence")
}
pub fn persistence_enabled(root: &Path) -> bool {
    persistence_path(root).is_file()
}
/// Exists while a volatile change waits for a client to confirm it. It survives a restart of
/// the service, so a crash (e.g. of Bluetooth) cannot leave an unconfirmed change behind.
pub fn pending_path(root: &Path) -> PathBuf {
    root.join("wifi-pending")
}
pub fn set_persistence(root: &Path, enabled: bool) -> std::io::Result<()> {
    let path = persistence_path(root);
    if enabled {
        std::fs::write(path, b"1\n")
    } else {
        match std::fs::remove_file(path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => Ok(()),
        }
    }
}

struct Task {
    owner: ClientKey,
    fingerprint: [u8; 32],
    state: WifiTaskState,
}
enum Work {
    Scan,
    Connect(String, Zeroizing<String>),
    AccessPoint(bool),
    Restart(RestartChange),
}
struct Job {
    id: u64,
    actor: ClientKey,
    work: Work,
}
type Tasks = Arc<Mutex<BTreeMap<u64, Task>>>;
pub struct WifiController {
    tasks: Tasks,
    sender: mpsc::SyncSender<Job>,
    registry: PathBuf,
    root: PathBuf,
    confirm: Arc<AtomicBool>,
    /// Kept up to date by the worker: the active configuration is not what applies after a restart.
    differs: Arc<AtomicBool>,
    access_point: bool,
}
impl WifiController {
    /// wpa_supplicant/dhcpcd fallback. `access_point` is the helper for addressing and DHCP;
    /// without it the device offers no access point.
    pub fn open(
        registry: &Path,
        interface: &str,
        local_directory: &Path,
        access_point: Option<Box<dyn AccessPointLink>>,
    ) -> Result<Self, WifiError> {
        validate_interface(interface)?;
        let socket = WpaSocket::open(
            &Path::new("/run/wpa_supplicant").join(interface),
            local_directory,
        )
        .map_err(|_| WifiError::Unavailable)?;
        let mut control = WpaNetworkControl::new(socket);
        if let Some(link) = access_point {
            control = control.with_access_point(link);
        }
        Self::launch(control, registry)
    }
    /// NetworkManager provides addressing and DHCP for the access point itself.
    #[cfg(all(target_os = "linux", feature = "network-manager"))]
    pub fn open_network_manager(registry: &Path, interface: &str) -> Result<Self, WifiError> {
        validate_interface(interface)?;
        let bus = super::network_manager::NetworkManagerBus::open(interface)
            .map_err(|_| WifiError::Unavailable)?;
        Self::launch(super::network_manager::NmNetworkControl::new(bus), registry)
    }
    fn launch<N: StackControl + Send + 'static>(
        control: N,
        registry: &Path,
    ) -> Result<Self, WifiError> {
        let store = FileStateStore::open(registry).map_err(|_| WifiError::Unavailable)?;
        let tasks = Arc::new(Mutex::new(BTreeMap::new()));
        let (sender, receiver) = mpsc::sync_channel(1);
        let journal = tasks.clone();
        let root = registry.parent().ok_or(WifiError::Unavailable)?.to_owned();
        let worker_root = root.clone();
        let confirm = Arc::new(AtomicBool::new(false));
        let worker_confirm = confirm.clone();
        let differs = Arc::new(AtomicBool::new(false));
        let worker_differs = differs.clone();
        let offers_access_point = control.access_point_supported();
        // The device owns individual credentials from its first start on; the
        // access point itself stays off until a client switches it on.
        if offers_access_point {
            let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
            if access_point::ensure_credentials(&root, hostname.trim(), |bytes| {
                getrandom::fill(bytes).is_ok()
            })
            .is_err()
            {
                eprintln!("Access point credentials could not be created");
            }
        }
        thread::Builder::new()
            .name("wifi-setup".into())
            .spawn(move || {
                run_worker(
                    control,
                    store,
                    receiver,
                    journal,
                    &worker_root,
                    &worker_confirm,
                    &worker_differs,
                );
            })
            .map_err(|_| WifiError::Unavailable)?;
        Ok(Self {
            tasks,
            sender,
            registry: registry.into(),
            root,
            confirm,
            differs,
            access_point: offers_access_point,
        })
    }
    fn authorize(&self, actor: &ClientKey) -> Result<(), WifiError> {
        FileStateStore::open(&self.registry)
            .and_then(|mut store| store.read())
            .map_err(|_| WifiError::Unavailable)?
            .registry
            .authorize(actor)
            .map_err(|_| WifiError::Unauthorized)
    }
    fn start(&mut self, actor: &ClientKey, id: u64, work: Work) -> Result<(), WifiError> {
        if id == 0 {
            return Err(WifiError::Conflict);
        }
        let mut digest = Sha256::new();
        match &work {
            Work::Connect(ssid, password) => {
                zorvinkel_application::setup::validate_wifi(ssid, password)
                    .map_err(|_| WifiError::Conflict)?;
                digest.update([1, ssid.len() as u8]);
                digest.update(ssid.as_bytes());
                digest.update(password.as_bytes());
            }
            Work::Scan => digest.update([0]),
            Work::AccessPoint(enabled) => {
                if !self.access_point {
                    return Err(WifiError::Unsupported);
                }
                digest.update([2, u8::from(*enabled)]);
            }
            Work::Restart(change) => {
                digest.update([
                    3,
                    match change {
                        RestartChange::LastAccepted => 0,
                        RestartChange::CurrentAsDefault => 1,
                        RestartChange::Revert => 2,
                    },
                ]);
            }
        }
        let fingerprint: [u8; 32] = digest.finalize().into();
        let mut tasks = self.tasks.lock().map_err(|_| WifiError::Unavailable)?;
        if let Some(task) = tasks.get(&id) {
            if task.owner != *actor {
                return Err(WifiError::Unauthorized);
            }
            return if task.fingerprint == fingerprint {
                Ok(())
            } else {
                Err(WifiError::Conflict)
            };
        }
        let state = FileStateStore::open(&self.registry)
            .and_then(|mut store| store.read())
            .map_err(|_| WifiError::Unavailable)?;
        state
            .registry
            .authorize(actor)
            .map_err(|_| WifiError::Unauthorized)?;
        // After a process restart, never execute an already journaled operation again.
        if state.operations.contains_key(&OperationId(id)) {
            return Err(WifiError::Conflict);
        }
        if tasks.len() >= 128
            || tasks
                .values()
                .any(|task| task.state == WifiTaskState::Pending)
        {
            return Err(WifiError::Busy);
        }
        tasks.insert(
            id,
            Task {
                owner: actor.clone(),
                fingerprint,
                state: WifiTaskState::Pending,
            },
        );
        let job = Job {
            id,
            actor: actor.clone(),
            work,
        };
        if self.sender.try_send(job).is_err() {
            tasks.remove(&id);
            return Err(WifiError::Unavailable);
        }
        Ok(())
    }
}
impl WifiSetupControl for WifiController {
    fn scan(&mut self, actor: &ClientKey, id: u64) -> Result<(), WifiError> {
        self.start(actor, id, Work::Scan)
    }
    fn connect(
        &mut self,
        actor: &ClientKey,
        id: u64,
        ssid: &str,
        password: &str,
    ) -> Result<(), WifiError> {
        self.start(
            actor,
            id,
            Work::Connect(ssid.into(), Zeroizing::new(password.into())),
        )
    }
    fn access_point_supported(&self) -> bool {
        self.access_point
    }
    fn access_point(
        &mut self,
        actor: &ClientKey,
    ) -> Result<Option<AccessPointSettings>, WifiError> {
        if !self.access_point {
            return Err(WifiError::Unsupported);
        }
        self.authorize(actor)?;
        access_point::read_credentials(&self.root).map_err(|_| WifiError::Unavailable)
    }
    fn set_access_point(
        &mut self,
        actor: &ClientKey,
        ssid: &str,
        password: &str,
    ) -> Result<(), WifiError> {
        if !self.access_point {
            return Err(WifiError::Unsupported);
        }
        zorvinkel_application::setup::validate_access_point(ssid, password)
            .map_err(|_| WifiError::Conflict)?;
        self.authorize(actor)?;
        // Credentials only change while no network task runs; an active access point keeps
        // its old ones until it is switched off and on again.
        if self
            .tasks
            .lock()
            .map_err(|_| WifiError::Unavailable)?
            .values()
            .any(|task| task.state == WifiTaskState::Pending)
        {
            return Err(WifiError::Busy);
        }
        access_point::write_credentials(&self.root, ssid, password)
            .map_err(|_| WifiError::Unavailable)
    }
    fn switch_access_point(
        &mut self,
        actor: &ClientKey,
        id: u64,
        enabled: bool,
    ) -> Result<(), WifiError> {
        self.start(actor, id, Work::AccessPoint(enabled))
    }
    fn poll(&mut self, actor: &ClientKey, id: u64) -> Result<WifiTaskState, WifiError> {
        if let Some(task) = self
            .tasks
            .lock()
            .map_err(|_| WifiError::Unavailable)?
            .get(&id)
        {
            if task.owner != *actor {
                return Err(WifiError::Unauthorized);
            }
            return Ok(task.state.clone());
        }
        let state = FileStateStore::open(&self.registry)
            .and_then(|mut store| store.read())
            .map_err(|_| WifiError::Unavailable)?;
        let operation = state
            .operations
            .get(&OperationId(id))
            .ok_or(WifiError::Unavailable)?;
        if operation.owner != *actor {
            return Err(WifiError::Unauthorized);
        }
        Ok(operation_state(&operation.status))
    }
    fn client_active(&mut self, _: &ClientKey) {
        self.confirm.store(true, Ordering::SeqCst);
        let _ = std::fs::remove_file(pending_path(&self.root));
    }
    fn restart_supported(&self) -> bool {
        true
    }
    fn restart_state(&mut self, actor: &ClientKey) -> Result<RestartState, WifiError> {
        let state = FileStateStore::open(&self.registry)
            .and_then(|mut store| store.read())
            .map_err(|_| WifiError::Unavailable)?;
        state
            .registry
            .authorize(actor)
            .map_err(|_| WifiError::Unauthorized)?;
        Ok(match state.settings.restart_policy {
            RestartPolicy::LastAccepted
            | RestartPolicy::DefaultProfile(NetworkConfiguration::Disconnected) => {
                RestartState::LastAccepted
            }
            RestartPolicy::DefaultProfile(NetworkConfiguration::Wifi(_)) => {
                RestartState::DefaultWifi
            }
            RestartPolicy::DefaultProfile(NetworkConfiguration::AccessPoint(_)) => {
                RestartState::DefaultAccessPoint
            }
        })
    }
    fn restart_differs(&mut self, _: &ClientKey) -> bool {
        self.differs.load(Ordering::SeqCst)
    }
    fn change_restart(
        &mut self,
        actor: &ClientKey,
        id: u64,
        change: RestartChange,
    ) -> Result<(), WifiError> {
        self.start(actor, id, Work::Restart(change))
    }
    fn persistence_supported(&self) -> bool {
        true
    }
    fn persistence(&mut self, actor: &ClientKey, change: Option<bool>) -> Result<bool, WifiError> {
        FileStateStore::open(&self.registry)
            .and_then(|mut store| store.read())
            .map_err(|_| WifiError::Unavailable)?
            .registry
            .authorize(actor)
            .map_err(|_| WifiError::Unauthorized)?;
        if let Some(enabled) = change {
            set_persistence(&self.root, enabled).map_err(|_| WifiError::Unavailable)?;
        }
        Ok(persistence_enabled(&self.root))
    }
}
fn validate_interface(interface: &str) -> Result<(), WifiError> {
    if interface.is_empty()
        || !interface
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
    {
        return Err(WifiError::Unavailable);
    }
    Ok(())
}
fn operation_state(status: &OperationStatus) -> WifiTaskState {
    match status {
        OperationStatus::Pending => WifiTaskState::Pending,
        OperationStatus::Succeeded { .. } => WifiTaskState::Connected,
        OperationStatus::Failed {
            rollback_failed: true,
            ..
        }
        | OperationStatus::NeedsReconciliation { .. } => WifiTaskState::RecoveryFailed,
        OperationStatus::Failed {
            cause: NetworkError::Rejected | NetworkError::TimedOut,
            ..
        } => WifiTaskState::Rejected,
        _ => WifiTaskState::Unavailable,
    }
}
/// A lost volatile connection is given up after this many failed checks in a row.
const LOSS_ATTEMPTS: u32 = 3;
const LOSS_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
/// Without an authenticated client talking to the device again within this time, a volatile
/// change is reverted: the client may have lost its Bluetooth access because of it.
const CONFIRM_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);
/// Tries to switch a kept access point on again after a restart, one try per worker tick,
/// because wpa_supplicant may still be starting.
const RESTORE_ATTEMPTS: u32 = 15;

/// Not journaled: no client asked for it, and it only repeats an accepted configuration.
fn restore_access_point(network: &mut impl StackControl, root: &Path) -> Result<(), ()> {
    let settings = access_point::read_credentials(root)
        .map_err(|_| ())?
        .ok_or(())?;
    let profile = network
        .prepare_access_point(&settings.ssid, &settings.password)
        .map_err(|_| ())?;
    if network
        .apply(&NetworkIntent::EnableAccessPoint(profile))
        .is_err()
    {
        // Drop the half-applied volatile state; wpa_supplicant reloads its saved networks.
        let _ = network.revert_volatile();
        return Err(());
    }
    network.discard_inactive_profiles().map_err(|_| ())
}
struct VolatileWatch {
    /// After a restart the own profiles are unknown, so only confirmation counts.
    recovered: bool,
    lost: u32,
    next: std::time::Instant,
    deadline: std::time::Instant,
}
impl VolatileWatch {
    fn new(now: std::time::Instant) -> Self {
        Self {
            recovered: false,
            lost: 0,
            next: now + LOSS_CHECK_INTERVAL,
            deadline: now + CONFIRM_WINDOW,
        }
    }
    fn recovered(now: std::time::Instant) -> Self {
        Self {
            recovered: true,
            ..Self::new(now)
        }
    }
    /// True when the volatile connection must be given up. `owned_active` is only asked when
    /// a check is due.
    fn due(
        &mut self,
        now: std::time::Instant,
        confirmed: bool,
        owned_active: impl FnOnce() -> Option<bool>,
    ) -> bool {
        if now < self.next {
            return false;
        }
        self.next = now + LOSS_CHECK_INTERVAL;
        match if self.recovered { None } else { owned_active() } {
            Some(true) => self.lost = 0,
            Some(false) => self.lost += 1,
            None => (),
        }
        self.lost >= LOSS_ATTEMPTS || (!confirmed && now >= self.deadline)
    }
}
/// What the device applies after a restart, as far as the worker needs to know.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Restart {
    LastAccepted,
    DefaultWifi(String),
    DefaultAccessPoint,
}
fn restart_of(store: &mut impl StateStore) -> Result<Restart, ()> {
    Ok(
        match store.read().map_err(|_| ())?.settings.restart_policy {
            RestartPolicy::LastAccepted
            | RestartPolicy::DefaultProfile(NetworkConfiguration::Disconnected) => {
                Restart::LastAccepted
            }
            RestartPolicy::DefaultProfile(NetworkConfiguration::Wifi(profile)) => {
                Restart::DefaultWifi(profile_ssid(&profile).ok_or(())?)
            }
            RestartPolicy::DefaultProfile(NetworkConfiguration::AccessPoint(_)) => {
                Restart::DefaultAccessPoint
            }
        },
    )
}
/// New connections and the access point only survive a restart when the client chose to keep
/// them and no default is set; a default is what comes back, so nothing else is kept.
fn kept(root: &Path, restart: &Restart) -> bool {
    *restart == Restart::LastAccepted && persistence_enabled(root)
}
/// Brings the configuration back that a client fixed as the default. Not
/// journaled: no client asked for it, and it only repeats an accepted configuration.
fn apply_restart_default(
    network: &mut impl StackControl,
    root: &Path,
    restart: &Restart,
) -> Result<(), ()> {
    match restart {
        Restart::LastAccepted => Ok(()),
        Restart::DefaultWifi(ssid) => network.apply_default_wifi(ssid).map_err(|_| ()),
        Restart::DefaultAccessPoint => {
            if matches!(network.observe(), Ok(NetworkConfiguration::AccessPoint(_))) {
                return Ok(());
            }
            restore_access_point(network, root)
        }
    }
}
/// What the worker restores on its own after it started.
enum Startup {
    Nothing,
    KeptAccessPoint,
    Default(Restart),
}
fn run_worker(
    mut network: impl StackControl,
    mut store: FileStateStore,
    receiver: mpsc::Receiver<Job>,
    tasks: Tasks,
    root: &Path,
    confirm: &AtomicBool,
    differs: &AtomicBool,
) {
    let mut reconciled = false;
    // A default is applied again; otherwise a kept access point is switched on again, and a
    // flag without the persistence choice is stale.
    let (startup, mut restore_attempts) = match restart_of(&mut store) {
        Ok(restart @ Restart::DefaultWifi(_)) => (Startup::Default(restart), RESTORE_ATTEMPTS),
        Ok(Restart::DefaultAccessPoint) if network.access_point_supported() => (
            Startup::Default(Restart::DefaultAccessPoint),
            RESTORE_ATTEMPTS,
        ),
        _ if network.access_point_supported() && access_point::restart_path(root).exists() => {
            if persistence_enabled(root) {
                (Startup::KeptAccessPoint, RESTORE_ATTEMPTS)
            } else {
                let _ = std::fs::remove_file(access_point::restart_path(root));
                (Startup::Nothing, 0)
            }
        }
        _ => (Startup::Nothing, 0),
    };
    // After a restart an unconfirmed change is only kept while a client shows up again.
    let mut watch = pending_path(root)
        .exists()
        .then(|| VolatileWatch::recovered(std::time::Instant::now()));
    loop {
        if !reconciled {
            reconciled = reconcile(&mut store, &mut network).is_ok();
        }
        let job = match receiver.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(job) => job,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if reconciled && restore_attempts > 0 {
                    restore_attempts -= 1;
                    let restored = match &startup {
                        Startup::Nothing => Ok(()),
                        Startup::KeptAccessPoint => restore_access_point(&mut network, root),
                        Startup::Default(restart) => {
                            apply_restart_default(&mut network, root, restart)
                        }
                    };
                    if restored.is_ok() {
                        restore_attempts = 0;
                    } else if restore_attempts == 0 {
                        eprintln!("Network configuration could not be restored after the restart");
                    }
                }
                if let Some(active) = &mut watch
                    && active.due(
                        std::time::Instant::now(),
                        confirm.load(Ordering::SeqCst),
                        // Errors reading the status are not evidence of a lost connection.
                        || network.owned_connection_active().ok(),
                    )
                {
                    revert_unconfirmed(&mut network, &mut store, root);
                    watch = None;
                }
                if reconciled {
                    publish_differs(&mut network, &mut store, differs);
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        watch = None;
        restore_attempts = 0;
        if !reconciled {
            reconciled = reconcile(&mut store, &mut network).is_ok();
        }
        let (result, volatile) = if reconciled {
            run_job(&mut network, &mut store, root, &job)
        } else {
            (WifiTaskState::Unavailable, false)
        };
        if volatile && result == WifiTaskState::Connected {
            confirm.store(false, Ordering::SeqCst);
            if std::fs::write(pending_path(root), b"1\n").is_ok() {
                watch = Some(VolatileWatch::new(std::time::Instant::now()));
            } else {
                eprintln!("Network change cannot be tracked for revert");
            }
        }
        // The comparison is current before the client polls the task that changed it.
        publish_differs(&mut network, &mut store, differs);
        if let Ok(mut tasks) = tasks.lock()
            && let Some(task) = tasks.get_mut(&job.id)
        {
            task.state = result;
        }
    }
}
/// Whether the active configuration differs from the one that applies after a restart: with a
/// default, when the active network is not the default; otherwise, while the connection is one
/// of the service's own volatile profiles. A failed reading leaves the last answer in place.
fn publish_differs(
    network: &mut impl StackControl,
    store: &mut impl StateStore,
    differs: &AtomicBool,
) {
    let answer = (|| {
        Some(match restart_of(store).ok()? {
            Restart::LastAccepted => network.owned_connection_active().ok()?,
            Restart::DefaultWifi(ssid) => {
                let profile = network.saved_wifi_profile(&ssid).ok()??;
                network.observe().ok()? != NetworkConfiguration::Wifi(profile)
            }
            Restart::DefaultAccessPoint => !matches!(
                network.observe().ok()?,
                NetworkConfiguration::AccessPoint(_)
            ),
        })
    })();
    if let Some(answer) = answer {
        differs.store(answer, Ordering::SeqCst);
    }
}
/// Back to what applies after a restart: the volatile profiles go, the saved configuration
/// returns, and so does the default where one is set.
fn revert_to_persisted(
    network: &mut impl StackControl,
    store: &mut impl StateStore,
    root: &Path,
) -> Result<(), ()> {
    let reverted = network.revert_volatile().map_err(|_| {
        eprintln!("Original network settings could not be restored");
    });
    let _ = std::fs::remove_file(pending_path(root));
    let restart = restart_of(store)?;
    apply_restart_default(network, root, &restart).map_err(|()| {
        eprintln!("Default network configuration could not be applied");
    })?;
    reverted
}
/// A volatile change nobody confirmed is given up.
fn revert_unconfirmed(network: &mut impl StackControl, store: &mut impl StateStore, root: &Path) {
    let _ = revert_to_persisted(network, store, root);
}
/// Executes one queued task; the flag says that the result stays volatile and needs a client to
/// confirm it.
fn run_job(
    network: &mut impl StackControl,
    store: &mut FileStateStore,
    root: &Path,
    job: &Job,
) -> (WifiTaskState, bool) {
    let mut volatile = false;
    let result = (|| {
        store
            .read()
            .map_err(|_| ())?
            .registry
            .authorize(&job.actor)
            .map_err(|_| ())?;
        let restart = restart_of(store)?;
        match &job.work {
            Work::Connect(ssid, password) => {
                let profile = network.prepare_wifi(ssid, password).map_err(|_| ())?;
                let operation = execute(
                    store,
                    network,
                    &job.actor,
                    OperationId(job.id),
                    NetworkIntent::ConnectWifi(profile),
                );
                let result = operation
                    .map(|operation| operation_state(&operation.status))
                    .map_err(|_| ());
                // Keep only an active owned profile; never delete foreign profiles.
                if network.discard_inactive_profiles().is_err() {
                    return Ok(WifiTaskState::RecoveryFailed);
                }
                if matches!(result, Ok(WifiTaskState::Connected)) {
                    // The access point is gone once the radio joined a network.
                    let _ = std::fs::remove_file(access_point::restart_path(root));
                    if kept(root, &restart) {
                        if network.persist_active().is_err() {
                            // The connection stays up; the profile remains volatile.
                            eprintln!("Wi-Fi profile could not be persisted");
                            volatile = true;
                        }
                    } else {
                        volatile = true;
                    }
                }
                result
            }
            Work::AccessPoint(enabled) => {
                // A pending revert would tear down the access point, so it ends here.
                let _ = std::fs::remove_file(pending_path(root));
                let intent = if *enabled {
                    let settings = access_point::read_credentials(root)
                        .map_err(|_| ())?
                        .ok_or(())?;
                    let profile = network
                        .prepare_access_point(&settings.ssid, &settings.password)
                        .map_err(|_| ())?;
                    NetworkIntent::EnableAccessPoint(profile)
                } else {
                    NetworkIntent::DisableAccessPoint
                };
                let operation = execute(store, network, &job.actor, OperationId(job.id), intent);
                let result = operation
                    .map(|operation| operation_state(&operation.status))
                    .map_err(|_| ());
                if network.discard_inactive_profiles().is_err() {
                    return Ok(WifiTaskState::RecoveryFailed);
                }
                if matches!(result, Ok(WifiTaskState::Connected)) {
                    let marker = access_point::restart_path(root);
                    if *enabled && kept(root, &restart) {
                        if std::fs::write(&marker, b"1\n").is_err() {
                            eprintln!("Access point cannot be kept across a restart");
                        }
                    } else {
                        let _ = std::fs::remove_file(&marker);
                        // Without a client that confirms it, the access point is given up again
                        // (unless it is the default that returns anyway).
                        volatile = *enabled && restart != Restart::DefaultAccessPoint;
                    }
                    // Without the access point the saved networks may be used again.
                    if !*enabled {
                        if network.reconnect().is_err() {
                            eprintln!("Saved Wi-Fi networks could not be rejoined");
                        }
                        if let Restart::DefaultWifi(_) = restart {
                            let _ = apply_restart_default(network, root, &restart);
                        }
                    }
                }
                result
            }
            Work::Restart(RestartChange::LastAccepted) => {
                set_restart_policy(store, &job.actor, RestartPolicy::LastAccepted)
                    .map_err(|_| ())?;
                Ok(WifiTaskState::Connected)
            }
            Work::Restart(RestartChange::Revert) => {
                revert_to_persisted(network, store, root)?;
                Ok(WifiTaskState::Connected)
            }
            Work::Restart(RestartChange::CurrentAsDefault) => {
                let active = network.observe().map_err(|_| ())?;
                match &active {
                    NetworkConfiguration::Wifi(_) => {
                        network.keep_active().map_err(|_| ())?;
                    }
                    // Only the device's own access point can come back; another one cannot.
                    NetworkConfiguration::AccessPoint(_) => {
                        if !network.owned_connection_active().map_err(|_| ())? {
                            return Ok(WifiTaskState::Rejected);
                        }
                        access_point::read_credentials(root)
                            .map_err(|_| ())?
                            .ok_or(())?;
                    }
                    NetworkConfiguration::Disconnected => return Ok(WifiTaskState::Rejected),
                }
                set_restart_policy(store, &job.actor, RestartPolicy::DefaultProfile(active))
                    .map_err(|_| ())?;
                // The default returns on its own, so nothing else waits for a confirmation.
                let _ = std::fs::remove_file(pending_path(root));
                let _ = std::fs::remove_file(access_point::restart_path(root));
                Ok(WifiTaskState::Connected)
            }
            Work::Scan => {
                let mut networks = network.scan().map_err(|_| ())?;
                networks.truncate(zorvinkel_protocol::wifi::MAX_NETWORKS);
                Ok(WifiTaskState::Networks(networks))
            }
        }
    })()
    .unwrap_or(WifiTaskState::Unavailable);
    (result, volatile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zorvinkel_application::{registration::register, state::DeviceState};
    #[test]
    fn queued_operations_are_deduplicated_owned_and_exclusive() {
        let root = std::env::temp_dir().join(format!(
            "zv-wifi-jobs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let registry = root.join("registry");
        let mut store = FileStateStore::initialize(
            &registry,
            DeviceState::initial(NetworkConfiguration::Disconnected),
        )
        .unwrap();
        let actor = ClientKey::new(vec![1; 32]).unwrap();
        register(&mut store, actor.clone(), 0).unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut wifi = WifiController {
            tasks: Arc::default(),
            sender,
            registry,
            root: root.clone(),
            confirm: Arc::default(),
            differs: Arc::default(),
            access_point: false,
        };
        wifi.connect(&actor, 41, "Event", "test-password").unwrap();
        wifi.connect(&actor, 41, "Event", "test-password").unwrap();
        assert_eq!(
            wifi.connect(&actor, 41, "Event", "another-password"),
            Err(WifiError::Conflict)
        );
        assert_eq!(wifi.scan(&actor, 42), Err(WifiError::Busy));
        assert_eq!(
            wifi.poll(&ClientKey::new(vec![2; 32]).unwrap(), 41),
            Err(WifiError::Unauthorized)
        );
        assert_eq!(wifi.poll(&actor, 41), Ok(WifiTaskState::Pending));
        let job = receiver.try_recv().unwrap();
        assert_eq!(job.id, 41);
        assert!(receiver.try_recv().is_err());
        wifi.tasks.lock().unwrap().get_mut(&41).unwrap().state = WifiTaskState::Connected;
        wifi.connect(&actor, 41, "Event", "test-password").unwrap();
        assert!(receiver.try_recv().is_err());
        assert!(store.read().unwrap().operations.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn volatile_change_is_reverted_when_lost_or_unconfirmed() {
        use std::time::{Duration, Instant};
        let start = Instant::now();
        let at = |seconds| start + Duration::from_secs(seconds);
        let mut watch = VolatileWatch::new(start);
        assert!(!watch.due(at(1), false, || panic!("not due yet")));
        assert!(!watch.due(at(5), true, || Some(true)));
        assert!(!watch.due(at(10), true, || Some(false)));
        assert!(!watch.due(at(15), true, || None));
        assert!(!watch.due(at(20), true, || Some(false)));
        assert!(watch.due(at(25), true, || Some(false)));
        let mut unconfirmed = VolatileWatch::new(start);
        assert!(!unconfirmed.due(at(55), false, || Some(true)));
        assert!(unconfirmed.due(at(60), false, || Some(true)));
        let mut confirmed = VolatileWatch::new(start);
        assert!(!confirmed.due(at(60), true, || Some(true)));
        let mut restarted = VolatileWatch::recovered(start);
        for seconds in [5, 10, 15, 20] {
            assert!(!restarted.due(at(seconds), false, || panic!("own profiles unknown")));
        }
        assert!(restarted.due(at(60), false, || panic!("own profiles unknown")));
    }
    #[test]
    fn persistence_choice_is_authorized_and_survives_reads() {
        let root = std::env::temp_dir().join(format!("zv-wifi-persist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let registry = root.join("registry");
        let mut store = FileStateStore::initialize(
            &registry,
            DeviceState::initial(NetworkConfiguration::Disconnected),
        )
        .unwrap();
        let actor = ClientKey::new(vec![1; 32]).unwrap();
        register(&mut store, actor.clone(), 0).unwrap();
        let (sender, _receiver) = mpsc::sync_channel(1);
        let mut wifi = WifiController {
            tasks: Arc::default(),
            sender,
            registry,
            root: root.clone(),
            confirm: Arc::default(),
            differs: Arc::default(),
            access_point: false,
        };
        assert!(wifi.persistence_supported());
        assert_eq!(wifi.persistence(&actor, None), Ok(false));
        assert_eq!(wifi.persistence(&actor, Some(true)), Ok(true));
        assert_eq!(wifi.persistence(&actor, None), Ok(true));
        assert_eq!(wifi.persistence(&actor, Some(false)), Ok(false));
        let stranger = ClientKey::new(vec![2; 32]).unwrap();
        assert_eq!(
            wifi.persistence(&stranger, Some(true)),
            Err(WifiError::Unauthorized)
        );
        assert!(!persistence_enabled(&root));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn access_point_credentials_are_authorized_private_and_changed_only_while_idle() {
        let root = std::env::temp_dir().join(format!("zv-wifi-ap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir(&root).unwrap();
        let registry = root.join("registry");
        let mut store = FileStateStore::initialize(
            &registry,
            DeviceState::initial(NetworkConfiguration::Disconnected),
        )
        .unwrap();
        let actor = ClientKey::new(vec![1; 32]).unwrap();
        register(&mut store, actor.clone(), 0).unwrap();
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut wifi = WifiController {
            tasks: Arc::default(),
            sender,
            registry,
            root: root.clone(),
            confirm: Arc::default(),
            differs: Arc::default(),
            access_point: true,
        };
        assert!(wifi.access_point_supported());
        assert!(wifi.access_point(&actor).unwrap().is_none());
        wifi.set_access_point(&actor, "Zorvinkel-Box", "password-ok")
            .unwrap();
        let stored = wifi.access_point(&actor).unwrap().unwrap();
        assert_eq!(
            (stored.ssid.as_str(), stored.password.as_str()),
            ("Zorvinkel-Box", "password-ok")
        );
        let stranger = ClientKey::new(vec![2; 32]).unwrap();
        assert_eq!(
            wifi.access_point(&stranger).err(),
            Some(WifiError::Unauthorized)
        );
        assert_eq!(
            wifi.set_access_point(&stranger, "Evil", "password-ok"),
            Err(WifiError::Unauthorized)
        );
        assert_eq!(
            wifi.set_access_point(&actor, "Box", "short"),
            Err(WifiError::Conflict)
        );
        // Switching is one exclusive, deduplicated task like every other network change.
        wifi.switch_access_point(&actor, 51, true).unwrap();
        wifi.switch_access_point(&actor, 51, true).unwrap();
        assert_eq!(
            wifi.switch_access_point(&actor, 51, false),
            Err(WifiError::Conflict)
        );
        assert_eq!(
            wifi.switch_access_point(&actor, 52, false),
            Err(WifiError::Busy)
        );
        assert_eq!(
            wifi.set_access_point(&actor, "Other", "password-ok"),
            Err(WifiError::Busy)
        );
        assert_eq!(wifi.poll(&actor, 51), Ok(WifiTaskState::Pending));
        assert!(matches!(
            receiver.try_recv().unwrap().work,
            Work::AccessPoint(true)
        ));
        assert!(receiver.try_recv().is_err());

        wifi.access_point = false;
        assert!(!wifi.access_point_supported());
        assert_eq!(
            wifi.access_point(&actor).err(),
            Some(WifiError::Unsupported)
        );
        assert_eq!(
            wifi.switch_access_point(&actor, 53, true),
            Err(WifiError::Unsupported)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A stack that only records what the worker asks of it.
    #[derive(Default)]
    struct Fake {
        current: Option<NetworkConfiguration>,
        saved: Vec<String>,
        owned_active: bool,
        log: Vec<String>,
        counter: u32,
    }
    fn profile(kind: &str, ssid: &str, id: u32) -> ProfileId {
        let hex = ssid
            .bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        ProfileId::new(format!("{kind}:{id}:{hex}")).unwrap()
    }
    impl zorvinkel_application::network::NetworkControl for Fake {
        fn check_support(&mut self, _: &NetworkIntent) -> Result<(), NetworkError> {
            Ok(())
        }
        fn observe(&mut self) -> Result<NetworkConfiguration, NetworkError> {
            Ok(self
                .current
                .clone()
                .unwrap_or(NetworkConfiguration::Disconnected))
        }
        fn apply(&mut self, intent: &NetworkIntent) -> Result<(), NetworkError> {
            self.log.push(format!("apply {intent:?}"));
            self.current = Some(match intent {
                NetworkIntent::ConnectWifi(profile) => NetworkConfiguration::Wifi(profile.clone()),
                NetworkIntent::EnableAccessPoint(profile) => {
                    self.owned_active = true;
                    NetworkConfiguration::AccessPoint(profile.clone())
                }
                NetworkIntent::DisableAccessPoint => NetworkConfiguration::Disconnected,
            });
            Ok(())
        }
        fn restore(&mut self, _: &NetworkConfiguration) -> Result<(), NetworkError> {
            Ok(())
        }
    }
    impl StackControl for Fake {
        fn access_point_supported(&self) -> bool {
            true
        }
        fn prepare_wifi(&mut self, ssid: &str, _: &str) -> Result<ProfileId, NetworkError> {
            self.counter += 1;
            self.owned_active = true;
            Ok(profile("wpa", ssid, self.counter))
        }
        fn prepare_access_point(&mut self, ssid: &str, _: &str) -> Result<ProfileId, NetworkError> {
            self.counter += 1;
            Ok(profile("wpa", ssid, self.counter))
        }
        fn scan(&mut self) -> Result<Vec<String>, NetworkError> {
            Ok(vec![])
        }
        fn discard_inactive_profiles(&mut self) -> Result<(), NetworkError> {
            Ok(())
        }
        fn persist_active(&mut self) -> Result<(), NetworkError> {
            self.log.push("persist".into());
            Ok(())
        }
        fn owned_connection_active(&mut self) -> Result<bool, NetworkError> {
            Ok(self.owned_active)
        }
        fn reconnect(&mut self) -> Result<(), NetworkError> {
            self.log.push("reconnect".into());
            Ok(())
        }
        fn revert_volatile(&mut self) -> Result<(), NetworkError> {
            self.log.push("revert".into());
            self.current = None;
            self.owned_active = false;
            Ok(())
        }
        fn saved_wifi_profile(&mut self, ssid: &str) -> Result<Option<ProfileId>, NetworkError> {
            Ok(self
                .saved
                .iter()
                .position(|saved| saved == ssid)
                .map(|index| profile("wpa", ssid, 100 + index as u32)))
        }
        fn keep_active(&mut self) -> Result<(), NetworkError> {
            self.log.push("keep".into());
            Ok(())
        }
        fn release_selection(&mut self) -> Result<(), NetworkError> {
            self.log.push("release".into());
            Ok(())
        }
    }
    struct Rig {
        root: PathBuf,
        store: FileStateStore,
        actor: ClientKey,
        fake: Fake,
    }
    impl Rig {
        fn new(name: &str) -> Self {
            let root =
                std::env::temp_dir().join(format!("zv-worker-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir(&root).unwrap();
            let mut store = FileStateStore::initialize(
                &root.join("registry"),
                DeviceState::initial(NetworkConfiguration::Disconnected),
            )
            .unwrap();
            let actor = ClientKey::new(vec![1; 32]).unwrap();
            register(&mut store, actor.clone(), 0).unwrap();
            access_point::write_credentials(&root, "Zorvinkel-Box", "password-ok").unwrap();
            Self {
                root,
                store,
                actor,
                fake: Fake::default(),
            }
        }
        fn run(&mut self, id: u64, work: Work) -> (WifiTaskState, bool) {
            let job = Job {
                id,
                actor: self.actor.clone(),
                work,
            };
            run_job(&mut self.fake, &mut self.store, &self.root, &job)
        }
        fn connect(&mut self, id: u64) -> (WifiTaskState, bool) {
            self.run(
                id,
                Work::Connect("Event".into(), Zeroizing::new("password-ok".into())),
            )
        }
        fn policy(&mut self) -> RestartPolicy {
            self.store.read().unwrap().settings.restart_policy
        }
    }
    impl Drop for Rig {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn new_connections_are_volatile_unless_kept_and_never_kept_next_to_a_default() {
        let mut rig = Rig::new("connect");
        assert_eq!(rig.connect(1), (WifiTaskState::Connected, true));
        assert!(!rig.fake.log.contains(&"persist".to_owned()));

        set_persistence(&rig.root, true).unwrap();
        assert_eq!(rig.connect(2), (WifiTaskState::Connected, false));
        assert!(rig.fake.log.contains(&"persist".to_owned()));

        // A default is what returns after a restart, so a newer connection is not kept.
        rig.fake.log.clear();
        let active = rig.fake.observe().unwrap();
        set_restart_policy(
            &mut rig.store,
            &rig.actor,
            RestartPolicy::DefaultProfile(active),
        )
        .unwrap();
        assert_eq!(rig.connect(3), (WifiTaskState::Connected, true));
        assert!(!rig.fake.log.contains(&"persist".to_owned()));
    }
    use zorvinkel_application::network::NetworkControl as _;

    #[test]
    fn the_access_point_is_volatile_unless_kept_or_the_default() {
        let mut rig = Rig::new("access-point");
        let marker = access_point::restart_path(&rig.root);
        assert_eq!(
            rig.run(1, Work::AccessPoint(true)),
            (WifiTaskState::Connected, true)
        );
        assert!(!marker.exists());

        set_persistence(&rig.root, true).unwrap();
        assert_eq!(
            rig.run(2, Work::AccessPoint(true)),
            (WifiTaskState::Connected, false)
        );
        assert!(marker.exists());
        assert_eq!(
            rig.run(3, Work::AccessPoint(false)),
            (WifiTaskState::Connected, false)
        );
        assert!(!marker.exists());
        assert!(rig.fake.log.contains(&"reconnect".to_owned()));

        // As the default the access point comes back anyway, so it is not given up.
        assert_eq!(
            rig.run(4, Work::AccessPoint(true)),
            (WifiTaskState::Connected, false)
        );
        assert_eq!(
            rig.run(5, Work::Restart(RestartChange::CurrentAsDefault)),
            (WifiTaskState::Connected, false)
        );
        assert!(matches!(
            rig.policy(),
            RestartPolicy::DefaultProfile(NetworkConfiguration::AccessPoint(_))
        ));
        assert!(!marker.exists());
        assert_eq!(
            rig.run(6, Work::AccessPoint(false)),
            (WifiTaskState::Connected, false)
        );
        assert_eq!(
            rig.run(7, Work::AccessPoint(true)),
            (WifiTaskState::Connected, false)
        );
    }

    #[test]
    fn the_current_configuration_becomes_the_default_and_can_be_dropped_again() {
        let mut rig = Rig::new("default");
        // Nothing to fix while the device is not connected.
        assert_eq!(
            rig.run(1, Work::Restart(RestartChange::CurrentAsDefault)),
            (WifiTaskState::Rejected, false)
        );
        assert_eq!(rig.policy(), RestartPolicy::LastAccepted);

        rig.connect(2);
        std::fs::write(pending_path(&rig.root), b"1\n").unwrap();
        assert_eq!(
            rig.run(3, Work::Restart(RestartChange::CurrentAsDefault)),
            (WifiTaskState::Connected, false)
        );
        assert!(rig.fake.log.contains(&"keep".to_owned()));
        let RestartPolicy::DefaultProfile(NetworkConfiguration::Wifi(fixed)) = rig.policy() else {
            panic!("default expected");
        };
        assert_eq!(profile_ssid(&fixed).as_deref(), Some("Event"));
        assert_eq!(
            restart_of(&mut rig.store),
            Ok(Restart::DefaultWifi("Event".into()))
        );
        // The default needs no confirmation any more.
        assert!(!pending_path(&rig.root).exists());

        // A foreign access point cannot be the default: its credentials are not ours.
        rig.fake.current = Some(NetworkConfiguration::AccessPoint(profile(
            "wpa", "Fremd", 9,
        )));
        rig.fake.owned_active = false;
        assert_eq!(
            rig.run(4, Work::Restart(RestartChange::CurrentAsDefault)),
            (WifiTaskState::Rejected, false)
        );

        assert_eq!(
            rig.run(5, Work::Restart(RestartChange::LastAccepted)),
            (WifiTaskState::Connected, false)
        );
        assert_eq!(rig.policy(), RestartPolicy::LastAccepted);
        // A stranger may not change the option.
        let job = Job {
            id: 6,
            actor: ClientKey::new(vec![2; 32]).unwrap(),
            work: Work::Restart(RestartChange::LastAccepted),
        };
        assert_eq!(
            run_job(&mut rig.fake, &mut rig.store, &rig.root, &job).0,
            WifiTaskState::Unavailable
        );
    }

    #[test]
    fn a_restart_applies_the_default_and_an_unconfirmed_change_returns_to_it() {
        let mut rig = Rig::new("apply");
        rig.fake.saved = vec!["Zuhause".into()];
        let wifi = Restart::DefaultWifi("Zuhause".into());
        apply_restart_default(&mut rig.fake, &rig.root, &wifi).unwrap();
        assert_eq!(
            rig.fake.log,
            [
                format!("apply ConnectWifi({:?})", profile("wpa", "Zuhause", 100)),
                "release".to_owned()
            ]
        );
        // Already on the default network: nothing is touched.
        rig.fake.log.clear();
        apply_restart_default(&mut rig.fake, &rig.root, &wifi).unwrap();
        assert!(rig.fake.log.is_empty());
        // The default network is gone from the saved configuration.
        assert!(
            apply_restart_default(
                &mut rig.fake,
                &rig.root,
                &Restart::DefaultWifi("Unbekannt".into())
            )
            .is_err()
        );

        // The access point as default comes back from the stored credentials.
        rig.fake.current = None;
        rig.fake.log.clear();
        apply_restart_default(&mut rig.fake, &rig.root, &Restart::DefaultAccessPoint).unwrap();
        assert!(rig.fake.log[0].starts_with("apply EnableAccessPoint"));
        rig.fake.log.clear();
        apply_restart_default(&mut rig.fake, &rig.root, &Restart::DefaultAccessPoint).unwrap();
        assert!(rig.fake.log.is_empty());
        assert!(apply_restart_default(&mut rig.fake, &rig.root, &Restart::LastAccepted).is_ok());

        // An unconfirmed volatile change is reverted and the default is applied again.
        rig.connect(1);
        set_restart_policy(
            &mut rig.store,
            &rig.actor,
            RestartPolicy::DefaultProfile(NetworkConfiguration::AccessPoint(profile(
                "wpa",
                "Zorvinkel-Box",
                5,
            ))),
        )
        .unwrap();
        std::fs::write(pending_path(&rig.root), b"1\n").unwrap();
        rig.fake.log.clear();
        revert_unconfirmed(&mut rig.fake, &mut rig.store, &rig.root);
        assert_eq!(rig.fake.log[0], "revert");
        assert!(rig.fake.log[1].starts_with("apply EnableAccessPoint"));
        assert!(!pending_path(&rig.root).exists());
    }

    #[test]
    fn the_comparison_and_the_revert_follow_what_applies_after_a_restart() {
        let mut rig = Rig::new("differs");
        let flag = AtomicBool::new(false);
        let differs = |rig: &mut Rig| {
            publish_differs(&mut rig.fake, &mut rig.store, &flag);
            flag.load(Ordering::SeqCst)
        };
        // Nothing changed yet.
        assert!(!differs(&mut rig));
        // A connection that was not kept is volatile: it differs, and a revert drops it at once.
        rig.connect(1);
        assert!(differs(&mut rig));
        rig.fake.log.clear();
        let (state, volatile) = rig.run(2, Work::Restart(RestartChange::Revert));
        assert_eq!((state, volatile), (WifiTaskState::Connected, false));
        assert_eq!(rig.fake.log[0], "revert");
        assert!(!differs(&mut rig));
        // With a saved network as default, the other network differs and a revert joins it.
        rig.fake.saved.push("Home".into());
        set_restart_policy(
            &mut rig.store,
            &rig.actor,
            RestartPolicy::DefaultProfile(NetworkConfiguration::Wifi(profile("wpa", "Home", 100))),
        )
        .unwrap();
        rig.connect(3);
        assert!(differs(&mut rig));
        rig.fake.log.clear();
        rig.run(4, Work::Restart(RestartChange::Revert));
        assert_eq!(rig.fake.log[0], "revert");
        assert!(rig.fake.log[1].starts_with("apply ConnectWifi"));
        assert!(!differs(&mut rig));
    }

    #[test]
    fn the_restart_option_is_readable_only_for_registered_clients() {
        let mut rig = Rig::new("state");
        let (sender, _receiver) = mpsc::sync_channel(1);
        let mut wifi = WifiController {
            tasks: Arc::default(),
            sender,
            registry: rig.root.join("registry"),
            root: rig.root.clone(),
            confirm: Arc::default(),
            differs: Arc::default(),
            access_point: true,
        };
        assert!(wifi.restart_supported());
        assert_eq!(
            wifi.restart_state(&rig.actor),
            Ok(RestartState::LastAccepted)
        );
        set_restart_policy(
            &mut rig.store,
            &rig.actor,
            RestartPolicy::DefaultProfile(NetworkConfiguration::Wifi(profile("wpa", "Event", 1))),
        )
        .unwrap();
        assert_eq!(
            wifi.restart_state(&rig.actor),
            Ok(RestartState::DefaultWifi)
        );
        let stranger = ClientKey::new(vec![2; 32]).unwrap();
        assert_eq!(wifi.restart_state(&stranger), Err(WifiError::Unauthorized));
        assert_eq!(
            wifi.change_restart(&stranger, 5, RestartChange::LastAccepted),
            Err(WifiError::Unauthorized)
        );
        wifi.change_restart(&rig.actor, 5, RestartChange::CurrentAsDefault)
            .unwrap();
        wifi.change_restart(&rig.actor, 5, RestartChange::CurrentAsDefault)
            .unwrap();
        assert_eq!(
            wifi.change_restart(&rig.actor, 5, RestartChange::LastAccepted),
            Err(WifiError::Conflict)
        );
        assert_eq!(wifi.poll(&rig.actor, 5), Ok(WifiTaskState::Pending));
    }
}
