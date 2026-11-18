//! State of one device: what the tile and the device tab show, and the requests it wants sent.
use super::i18n::Text;
use std::time::Duration;
use zorvinkel_application::{power::PowerAction, services::ServiceAction, setup::*};

/// The session is renewed this long before the secure profile would end it.
const RENEW_BEFORE: Duration = Duration::from_secs(45);
/// From here on a session without automatic renewal shows a notice.
const WARN_BEFORE: Duration = Duration::from_secs(120);

/// A hint about the device, kept as an identifier so that it follows the language.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Notice {
    FallbackToBluetooth,
    Cancelled,
    ShuttingDown,
    Restarting,
    EndedBySearch,
    TakenOver,
}
impl Notice {
    pub fn text(self) -> Text {
        match self {
            Self::FallbackToBluetooth => Text::NoticeFallbackToBluetooth,
            Self::Cancelled => Text::NoticeCancelled,
            Self::ShuttingDown => Text::NoticeShuttingDown,
            Self::Restarting => Text::NoticeRestarting,
            Self::EndedBySearch => Text::NoticeEndedBySearch,
            Self::TakenOver => Text::NoticeTakenOver,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    /// Joining a network, the device's own Wi-Fi and what happens after a restart.
    Wifi,
    System,
    /// Services of the group named by `Session::service_group`.
    Services,
    Power,
}
/// What the Wi-Fi page is about: joining another network or offering the device's own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WifiMode {
    Join,
    OwnNetwork,
}
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    Connect,
    Disconnect,
    Scan,
    Wifi,
    Credentials,
    Persistence,
    Power(PowerAction),
    Service(ServiceAction),
    AccessPoint,
    AccessPointSettings,
    RestartLast,
    RestartDefault,
    Refresh,
}
impl Action {
    /// Discovery belongs to the device list, not to a device.
    fn for_request(request: &SetupRequest) -> Option<Self> {
        Some(match request {
            SetupRequest::Discover => return None,
            SetupRequest::Connect(_) => Self::Connect,
            SetupRequest::Disconnect => Self::Disconnect,
            SetupRequest::Scan => Self::Scan,
            SetupRequest::ConnectWifi { .. } => Self::Wifi,
            SetupRequest::ReadSavedCredentials { .. }
            | SetupRequest::ReadCachedCredentials { .. } => Self::Credentials,
            SetupRequest::SetWifiPersistence(_) => Self::Persistence,
            SetupRequest::PowerDevice(action) => Self::Power(*action),
            SetupRequest::ServiceAction { action, .. } => Self::Service(*action),
            SetupRequest::EnableAccessPoint(_) | SetupRequest::DisableAccessPoint => {
                Self::AccessPoint
            }
            SetupRequest::UpdateAccessPoint(_) => Self::AccessPointSettings,
            SetupRequest::RestartLastAccepted => Self::RestartLast,
            SetupRequest::RestartCurrentAsDefault => Self::RestartDefault,
            SetupRequest::Refresh => Self::Refresh,
        })
    }
    /// The full sentence of the progress overlay.
    pub fn progress_title(self) -> Text {
        match self {
            Self::Connect => Text::ProgressConnect,
            Self::Disconnect => Text::ProgressDisconnect,
            Self::Scan => Text::ProgressScan,
            Self::Wifi => Text::ProgressWifi,
            Self::Credentials => Text::ProgressCredentials,
            Self::Persistence => Text::ProgressPersistence,
            Self::Power(PowerAction::Shutdown) => Text::ProgressShutdown,
            Self::Power(PowerAction::Restart) => Text::ProgressRestart,
            Self::Service(ServiceAction::Start) => Text::ProgressServiceStart,
            Self::Service(ServiceAction::Stop) => Text::ProgressServiceStop,
            Self::Service(ServiceAction::Restart) => Text::ProgressServiceRestart,
            Self::AccessPoint => Text::ProgressAccessPoint,
            Self::AccessPointSettings => Text::ProgressAccessPointSettings,
            Self::RestartLast | Self::RestartDefault => Text::ProgressRestartOption,
            Self::Refresh => Text::ProgressRefresh,
        }
    }
    /// The short state text of tiles and tabs.
    pub fn label(self) -> Text {
        match self {
            Self::Connect => Text::LabelConnect,
            Self::Disconnect => Text::LabelDisconnect,
            Self::Scan => Text::LabelScan,
            Self::Wifi => Text::LabelWifi,
            Self::Credentials => Text::LabelCredentials,
            Self::Persistence => Text::LabelPersistence,
            Self::Power(PowerAction::Shutdown) => Text::LabelShutdown,
            Self::Power(PowerAction::Restart) => Text::LabelRestart,
            Self::Service(ServiceAction::Start) => Text::LabelServiceStart,
            Self::Service(ServiceAction::Stop) => Text::LabelServiceStop,
            Self::Service(ServiceAction::Restart) => Text::LabelServiceRestart,
            Self::AccessPoint => Text::LabelAccessPoint,
            Self::AccessPointSettings => Text::LabelAccessPointSettings,
            Self::RestartLast | Self::RestartDefault => Text::LabelRestartOption,
            Self::Refresh => Text::LabelRefresh,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConnectionState {
    Disconnected,
    Connected,
    /// A request is running; the action names it.
    Working(Action),
    /// The last request failed and the device is not connected.
    Failed,
    /// The device was not found by the latest search.
    OutOfRange,
}

pub struct Session {
    pub handle: String,
    pub name: String,
    pub in_range: bool,
    pub tab_open: bool,
    pub simulation: bool,
    pub snapshot: SetupSnapshot,
    pub active_action: Option<Action>,
    /// A background status refresh is queued or running; it shows no activity.
    pub refresh_pending: bool,
    pub last_refresh: f64,
    pub page: Page,
    /// `None` until the user chooses; then it follows the device (own Wi-Fi while it is on).
    pub wifi_mode: Option<WifiMode>,
    pub service_group: String,
    /// Waits for the user's confirmation before the device is switched off or restarted.
    pub confirm_power: Option<PowerAction>,
    /// Waits for the user's confirmation before a service is stopped or restarted.
    pub confirm_service: Option<(String, ServiceAction)>,
    pub ssid: String,
    pub password: String,
    pub show_wifi_password: bool,
    pub ap_ssid: String,
    pub ap_password: String,
    pub show_ap_password: bool,
    pub notice: Option<Notice>,
    pub error: Option<SetupError>,
    /// Renews the secured session before it ends instead of letting the device disconnect.
    pub auto_renew: bool,
    outbox: Vec<SetupRequest>,
}

impl Session {
    pub fn new(handle: String, name: String, simulation: bool) -> Self {
        Self {
            handle,
            name,
            in_range: true,
            tab_open: false,
            simulation,
            snapshot: SetupSnapshot::default(),
            active_action: None,
            refresh_pending: false,
            last_refresh: 0.0,
            page: Page::Wifi,
            wifi_mode: None,
            service_group: String::new(),
            confirm_power: None,
            confirm_service: None,
            ssid: String::new(),
            password: String::new(),
            show_wifi_password: false,
            ap_ssid: String::new(),
            ap_password: String::new(),
            show_ap_password: true,
            notice: None,
            error: None,
            auto_renew: true,
            outbox: Vec::new(),
        }
    }
    pub fn is_connected(&self) -> bool {
        self.snapshot.status.is_some()
    }
    pub fn state(&self) -> ConnectionState {
        if let Some(action) = self.active_action {
            ConnectionState::Working(action)
        } else if self.is_connected() {
            ConnectionState::Connected
        } else if self.error.is_some() {
            ConnectionState::Failed
        } else if !self.in_range {
            ConnectionState::OutOfRange
        } else {
            ConnectionState::Disconnected
        }
    }
    /// Queues a request and marks the device as busy until its reply arrives.
    pub fn submit(&mut self, request: SetupRequest) {
        let Some(action) = Action::for_request(&request) else {
            return;
        };
        if matches!(request, SetupRequest::Connect(_) | SetupRequest::Disconnect) {
            self.password.clear();
        }
        self.error = None;
        self.active_action = Some(action);
        self.outbox.push(request);
    }
    pub fn connect(&mut self) {
        self.submit(SetupRequest::Connect(self.handle.clone()));
    }
    pub fn disconnect(&mut self) {
        self.submit(SetupRequest::Disconnect);
    }
    /// A connected device whose secured session is about to end and should be renewed now.
    pub fn needs_renewal(&self) -> bool {
        self.auto_renew
            && self.is_connected()
            && self.error.is_none()
            && self.active_action.is_none()
            && !self.refresh_pending
            && self
                .snapshot
                .session_remaining
                .is_some_and(|left| left <= RENEW_BEFORE)
    }
    /// Seconds left when the session ends soon and nothing renews it.
    pub fn expiry_warning(&self) -> Option<u64> {
        let left = self.snapshot.session_remaining?;
        (!self.auto_renew && self.is_connected() && left <= WARN_BEFORE).then_some(left.as_secs())
    }
    /// Text for the path the device is reached over.
    pub fn path_text(&self) -> Option<Text> {
        match self.snapshot.transport {
            Some(Transport::Ble) => Some(Text::PathBluetooth),
            Some(Transport::Ip) => Some(Text::PathNetwork),
            None if self.simulation && self.is_connected() => Some(Text::PathSimulation),
            None => None,
        }
    }
    pub fn take_outbox(&mut self) -> Vec<SetupRequest> {
        std::mem::take(&mut self.outbox)
    }
    /// Forgets the connection after the backend ended it, e.g. because another device took over.
    pub fn mark_disconnected(&mut self, reason: Notice) {
        self.snapshot = SetupSnapshot::default();
        self.notice = Some(reason);
    }
    /// Applies a reply; returns the action it finished, `None` for background refreshes.
    pub fn receive(
        &mut self,
        result: Result<SetupUpdate, SetupError>,
        cancelled: bool,
        background: bool,
    ) -> Option<Action> {
        let finished = if background {
            self.refresh_pending = false;
            None
        } else {
            self.active_action.take()
        };
        let previous_path = self.snapshot.transport;
        match result {
            Ok(update) => {
                if self.snapshot.access_point.is_none()
                    && let Some(ap) = &update.snapshot.access_point
                {
                    self.ap_ssid = ap.ssid.clone();
                    self.ap_password = ap.password.clone();
                }
                if let Some(password) = update.saved_password {
                    self.password = password;
                }
                self.snapshot = update.snapshot;
                self.snapshot.devices.clear();
                if !background || update.error.is_some() {
                    self.error = update.error;
                }
                if !background {
                    self.notice = self.notice_for(finished, cancelled);
                }
                if previous_path == Some(Transport::Ip)
                    && self.snapshot.transport == Some(Transport::Ble)
                {
                    self.notice = Some(Notice::FallbackToBluetooth);
                } else if self.snapshot.transport == Some(Transport::Ip)
                    && self.notice == Some(Notice::FallbackToBluetooth)
                {
                    // The session moved back to the network; the hint would contradict the path.
                    self.notice = None;
                }
            }
            Err(error) => {
                self.error = Some(error);
                self.snapshot.status = None;
                self.snapshot.access_point = None;
            }
        }
        finished
    }
    fn notice_for(&self, finished: Option<Action>, cancelled: bool) -> Option<Notice> {
        if cancelled {
            return Some(Notice::Cancelled);
        }
        match (finished, self.error) {
            (Some(Action::Power(PowerAction::Shutdown)), None) => Some(Notice::ShuttingDown),
            (Some(Action::Power(PowerAction::Restart)), None) => Some(Notice::Restarting),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zorvinkel_adapters::simulation::setup::SimulatedSetup;

    fn session() -> Session {
        Session::new("simulation:device-1".into(), "Fotobox 01".into(), false)
    }
    fn run(request: SetupRequest) -> Result<SetupUpdate, SetupError> {
        let mut backend = SimulatedSetup::default().discovered();
        backend.handle(request)
    }

    #[test]
    fn submitting_marks_the_device_busy_and_queues_the_request() {
        let mut session = session();
        session.connect();
        assert_eq!(session.state(), ConnectionState::Working(Action::Connect));
        let requests = session.take_outbox();
        assert!(
            matches!(&requests[..], [SetupRequest::Connect(handle)] if handle == &session.handle)
        );
        assert!(session.take_outbox().is_empty());
    }
    #[test]
    fn a_connect_reply_connects_and_a_failed_one_shows_the_error() {
        let mut session = session();
        session.connect();
        let finished = session.receive(
            run(SetupRequest::Connect(session.handle.clone())),
            false,
            false,
        );
        assert_eq!(finished, Some(Action::Connect));
        assert_eq!(session.state(), ConnectionState::Connected);
        assert!(session.snapshot.devices.is_empty());

        session.disconnect();
        session.receive(Err(SetupError::Unavailable), false, false);
        assert_eq!(session.state(), ConnectionState::Failed);
    }
    #[test]
    fn background_refresh_keeps_notice_and_shows_no_activity() {
        let mut session = session();
        session.notice = Some(Notice::Cancelled);
        session.refresh_pending = true;
        assert_eq!(session.state(), ConnectionState::Disconnected);
        let finished = session.receive(run(SetupRequest::Refresh), false, true);
        assert_eq!(finished, None);
        assert!(!session.refresh_pending);
        assert_eq!(session.notice, Some(Notice::Cancelled));
        assert!(session.error.is_none());
    }
    #[test]
    fn devices_missing_from_the_latest_search_are_out_of_range_unless_connected() {
        let mut session = session();
        session.in_range = false;
        assert_eq!(session.state(), ConnectionState::OutOfRange);
        session.receive(
            run(SetupRequest::Connect(session.handle.clone())),
            false,
            false,
        );
        assert_eq!(session.state(), ConnectionState::Connected);
    }
    fn connected_over(transport: Transport, remaining: Option<Duration>) -> Session {
        let mut session = session();
        let mut update = run(SetupRequest::Connect(session.handle.clone())).unwrap();
        update.snapshot.transport = Some(transport);
        update.snapshot.session_remaining = remaining;
        session.receive(Ok(update), false, false);
        session
    }
    #[test]
    fn a_session_is_renewed_shortly_before_it_ends_unless_the_user_opted_out() {
        let mut session = connected_over(Transport::Ip, Some(Duration::from_secs(300)));
        assert!(!session.needs_renewal());
        session.snapshot.session_remaining = Some(RENEW_BEFORE);
        assert!(session.needs_renewal());
        session.refresh_pending = true;
        assert!(!session.needs_renewal());
        session.refresh_pending = false;
        session.auto_renew = false;
        assert!(!session.needs_renewal());
        assert_eq!(session.expiry_warning(), Some(RENEW_BEFORE.as_secs()));
        session.snapshot.session_remaining = Some(Duration::from_secs(500));
        assert_eq!(session.expiry_warning(), None);
    }
    #[test]
    fn the_connection_path_is_named_and_a_switch_to_bluetooth_is_announced() {
        let mut session = connected_over(Transport::Ip, None);
        assert_eq!(session.path_text(), Some(Text::PathNetwork));
        let mut update = run(SetupRequest::Refresh).unwrap();
        update.snapshot.transport = Some(Transport::Ble);
        session.receive(Ok(update), false, true);
        assert_eq!(session.path_text(), Some(Text::PathBluetooth));
        assert_eq!(session.notice, Some(Notice::FallbackToBluetooth));
    }
    #[test]
    fn discovery_is_not_a_device_request() {
        let mut session = session();
        session.submit(SetupRequest::Discover);
        assert!(session.active_action.is_none());
        assert!(session.take_outbox().is_empty());
    }
    #[test]
    fn power_notice_follows_a_successful_power_action_only() {
        let mut session = session();
        session.submit(SetupRequest::PowerDevice(PowerAction::Restart));
        session.receive(run(SetupRequest::Refresh), false, false);
        assert_eq!(session.notice, Some(Notice::Restarting));
        session.submit(SetupRequest::PowerDevice(PowerAction::Restart));
        session.receive(Err(SetupError::Unavailable), false, false);
        assert_eq!(session.state(), ConnectionState::Failed);
    }
}
