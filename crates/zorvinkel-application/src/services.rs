//! Configured services of the device that clients may display. Authorization is the caller's duty.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceState {
    Unknown,
    Active,
    Activating,
    Inactive,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebScheme {
    Http,
    Https,
}

/// Web page of a service. The host is chosen by the client from the device's addresses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebLink {
    /// Caption of the link; the client shows its own text when absent.
    pub label: Option<String>,
    pub scheme: WebScheme,
    pub port: u16,
    pub path: String,
}
impl WebLink {
    /// URL of the page on `host`; IPv6 literals are bracketed.
    pub fn url(&self, host: std::net::IpAddr) -> String {
        let scheme = match self.scheme {
            WebScheme::Http => "http",
            WebScheme::Https => "https",
        };
        let host = match host {
            std::net::IpAddr::V4(address) => address.to_string(),
            std::net::IpAddr::V6(address) => format!("[{address}]"),
        };
        format!("{scheme}://{host}:{}{}", self.port, self.path)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceEntry {
    /// Tab of the device page the service is listed under.
    pub group: String,
    pub name: String,
    pub version: Option<String>,
    pub state: ServiceState,
    /// Actions the operator allowed for this service.
    pub actions: Vec<ServiceAction>,
    pub web: Option<WebLink>,
}

pub trait ServiceInfoSource {
    fn read(&mut self) -> Result<Vec<ServiceEntry>, crate::PortError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceControlError {
    /// No configured service has this name.
    Unknown,
    /// The configuration of the service does not allow the action.
    NotAllowed,
    /// Earlier requests are still waiting.
    Busy,
    Unavailable,
}
/// Starts, stops and restarts configured services. Authorization is the caller's duty.
pub trait ServiceControl {
    /// Queues the action; the service state changes afterwards and shows in the next reading.
    fn perform(&mut self, name: &str, action: ServiceAction) -> Result<(), ServiceControlError>;
}
