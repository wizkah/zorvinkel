//! Services the operator configured in `services.d` (ADR-021): state from systemd, version from a command.
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use zorvinkel_application::{
    PortError,
    services::{
        ServiceAction, ServiceControl, ServiceControlError, ServiceEntry, ServiceInfoSource,
        ServiceState, WebLink, WebScheme,
    },
};
use zorvinkel_protocol::services::{MAX_NAME, MAX_PATH, MAX_SERVICES, MAX_VERSION};

pub const CONFIG_DIRECTORY: &str = "/etc/zorvinkel/services.d";
/// Below the service's runtime directory; the helper's path unit watches it.
pub const REQUEST_DIRECTORY: &str = "service-requests";
const DEFAULT_GROUP: &str = "Dienste";
const MAX_FILE_BYTES: u64 = 4096;
const MAX_COMMAND_ARGUMENTS: usize = 8;
const MAX_ARGUMENT_BYTES: usize = 128;
const MAX_UNIT_BYTES: usize = 100;
/// A version rarely changes; the cache spares the device repeated process starts.
const VERSION_TTL: Duration = Duration::from_secs(600);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_COMMAND_OUTPUT: u64 = 1024;
/// Requests the root helper has not yet taken; more are refused so the queue stays small.
const MAX_PENDING_REQUESTS: usize = 8;
const MAX_REQUEST_BYTES: u64 = 256;
const REQUEST_UNITS: [&str; 1] = ["zorvinkel-service.path"];
/// Units an action could lock the operator out with; they may be shown but never controlled.
const PROTECTED_UNITS: [&str; 9] = [
    "bluetooth.service",
    "dhcpcd.service",
    "networking.service",
    "NetworkManager.service",
    "ssh.service",
    "sshd.service",
    "wpa_supplicant.service",
    "dbus.service",
    "rescue.service",
];
const PROTECTED_PREFIXES: [&str; 4] = ["zorvinkel", "systemd-", "getty@", "serial-getty@"];

/// One file of `services.d`; unknown keys are errors so typos do not go unnoticed.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceFile {
    name: String,
    group: Option<String>,
    unit: String,
    version_command: Option<Vec<String>>,
    /// 1-based word of the command's first output line that holds the version.
    version_word: Option<usize>,
    actions: Option<Vec<String>>,
    web: Option<WebSection>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WebSection {
    label: Option<String>,
    scheme: Option<String>,
    port: u16,
    path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConfiguredService {
    group: String,
    name: String,
    unit: String,
    version_command: Option<Vec<String>>,
    version_word: Option<usize>,
    actions: Vec<ServiceAction>,
    web: Option<WebLink>,
}

fn plain_text(text: &str, max: usize) -> bool {
    !text.is_empty() && text.len() <= max && !text.chars().any(char::is_control)
}
fn unit_valid(unit: &str) -> bool {
    let mut characters = unit.chars();
    unit.len() <= MAX_UNIT_BYTES
        && unit.ends_with(".service")
        // A leading character other than a letter or digit could pass as a `systemctl` option.
        && characters.next().is_some_and(|first| first.is_ascii_alphanumeric())
        && characters.all(|c| c.is_ascii_alphanumeric() || ":_.@-".contains(c))
}

fn parse_service(text: &str) -> Result<ConfiguredService, String> {
    let file: ServiceFile = toml::from_str(text).map_err(|error| error.message().to_owned())?;
    let group = file.group.unwrap_or_else(|| DEFAULT_GROUP.into());
    if !plain_text(&file.name, MAX_NAME) || !plain_text(&group, MAX_NAME) {
        return Err(format!(
            "name and group need 1 to {MAX_NAME} printable characters"
        ));
    }
    if !unit_valid(&file.unit) {
        return Err("unit must be a systemd service name such as raspotify.service".into());
    }
    if let Some(command) = &file.version_command
        && (command.is_empty()
            || command.len() > MAX_COMMAND_ARGUMENTS
            || command
                .iter()
                .any(|argument| argument.len() > MAX_ARGUMENT_BYTES || argument.contains('\0'))
            || command[0].is_empty())
    {
        return Err(format!(
            "version_command needs 1 to {MAX_COMMAND_ARGUMENTS} arguments of at most {MAX_ARGUMENT_BYTES} bytes"
        ));
    }
    if file.version_word == Some(0) {
        return Err("version_word counts from 1".into());
    }
    let actions = parse_actions(file.actions.as_deref().unwrap_or_default(), &file.unit)?;
    Ok(ConfiguredService {
        actions,
        group,
        name: file.name,
        unit: file.unit,
        version_command: file.version_command,
        version_word: file.version_word,
        web: file.web.map(parse_web).transpose()?,
    })
}
fn parse_actions(names: &[String], unit: &str) -> Result<Vec<ServiceAction>, String> {
    let mut actions = Vec::new();
    for name in names {
        let action = match name.as_str() {
            "start" => ServiceAction::Start,
            "stop" => ServiceAction::Stop,
            "restart" => ServiceAction::Restart,
            _ => return Err("actions may contain start, stop and restart".into()),
        };
        if actions.contains(&action) {
            return Err("actions lists an action twice".into());
        }
        actions.push(action);
    }
    if !actions.is_empty() && protected_unit(unit) {
        return Err(format!("{unit} is protected and cannot be controlled"));
    }
    Ok(actions)
}
fn protected_unit(unit: &str) -> bool {
    PROTECTED_UNITS.contains(&unit)
        || PROTECTED_PREFIXES
            .iter()
            .any(|prefix| unit.starts_with(prefix))
}
fn parse_web(web: WebSection) -> Result<WebLink, String> {
    let scheme = match web.scheme.as_deref().unwrap_or("http") {
        "http" => WebScheme::Http,
        "https" => WebScheme::Https,
        _ => return Err("web.scheme must be http or https".into()),
    };
    let path = web.path.unwrap_or_else(|| "/".into());
    if web.port == 0
        || path.len() > MAX_PATH
        || !path.starts_with('/')
        || !path.chars().all(|c| c.is_ascii_graphic())
    {
        return Err(format!(
            "web needs a port and a path of printable ASCII without spaces, starting with / (at most {MAX_PATH} bytes)"
        ));
    }
    if web
        .label
        .as_deref()
        .is_some_and(|label| !plain_text(label, MAX_NAME))
    {
        return Err(format!(
            "web.label needs 1 to {MAX_NAME} printable characters"
        ));
    }
    Ok(WebLink {
        label: web.label,
        scheme,
        port: web.port,
        path,
    })
}

/// Maps the blocks of `systemctl show -p LoadState -p ActiveState <units>` to states, in request order.
fn parse_states(output: &str, expected: usize) -> Vec<ServiceState> {
    let states = output
        .split("\n\n")
        .filter(|block| !block.trim().is_empty())
        .map(|block| {
            let value = |key: &str| {
                block
                    .lines()
                    .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            };
            match (value("LoadState"), value("ActiveState")) {
                (Some("not-found"), _) => ServiceState::Unknown,
                (_, Some("active")) => ServiceState::Active,
                (_, Some("activating" | "reloading")) => ServiceState::Activating,
                (_, Some("inactive" | "deactivating")) => ServiceState::Inactive,
                (_, Some("failed")) => ServiceState::Failed,
                _ => ServiceState::Unknown,
            }
        })
        .collect::<Vec<_>>();
    if states.len() == expected {
        states
    } else {
        vec![ServiceState::Unknown; expected]
    }
}

fn version_from_output(output: &str, word: Option<usize>) -> Option<String> {
    let line = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let version = match word {
        Some(index) => line.split_whitespace().nth(index - 1)?,
        None => line,
    };
    plain_text(version, MAX_VERSION).then(|| version.to_owned())
}

/// Runs a command without a shell, with a time and output limit; any failure means "unknown".
fn command_output(command: &[String]) -> Option<String> {
    let mut child = Command::new(&command[0])
        .args(&command[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if started.elapsed() >= COMMAND_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    let mut output = String::new();
    child
        .stdout
        .take()?
        .take(MAX_COMMAND_OUTPUT)
        .read_to_string(&mut output)
        .ok()?;
    Some(output)
}

/// Reads every valid `*.toml` of `directory`; the second value lists why other files were left out.
fn load_directory(directory: &Path) -> Result<(Vec<ConfiguredService>, Vec<String>), PortError> {
    let mut paths = fs::read_dir(directory)
        .map_err(|_| PortError::Unavailable)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "toml"))
        .filter(|entry| {
            entry
                .metadata()
                .is_ok_and(|meta| meta.is_file() && meta.len() <= MAX_FILE_BYTES)
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    paths.sort();
    let (mut services, mut problems) = (Vec::<ConfiguredService>::new(), Vec::new());
    for path in paths {
        let file = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let parsed = fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| parse_service(&text));
        match parsed {
            Ok(_) if services.len() == MAX_SERVICES => {
                problems.push(format!(
                    "{file}: more than {MAX_SERVICES} services, ignored"
                ));
            }
            // Requests name a service, so the name has to identify one.
            Ok(service) if services.iter().any(|other| other.name == service.name) => {
                problems.push(format!("{file}: name {:?} is already used", service.name));
            }
            Ok(service) => services.push(service),
            Err(error) => problems.push(format!("{file}: {error}")),
        }
    }
    Ok((services, problems))
}

pub struct ServiceReader {
    directory: PathBuf,
    /// Allowed actions are only reported where the root helper can perform them.
    report_actions: bool,
    versions: BTreeMap<Vec<String>, (Instant, Option<String>)>,
    /// Problems are logged when they change, not on every refresh.
    reported_problems: Vec<String>,
}
impl ServiceReader {
    pub fn new(directory: &Path, report_actions: bool) -> Self {
        Self {
            directory: directory.to_owned(),
            report_actions,
            versions: BTreeMap::new(),
            reported_problems: Vec::new(),
        }
    }
    /// The device offers the list once the configuration directory exists.
    pub fn configured(&self) -> bool {
        self.directory.is_dir()
    }
    fn load(&mut self) -> Result<Vec<ConfiguredService>, PortError> {
        let (services, problems) = load_directory(&self.directory)?;
        if problems != self.reported_problems {
            for problem in &problems {
                eprintln!("Service configuration ignored: {problem}");
            }
            self.reported_problems = problems;
        }
        Ok(services)
    }
    fn version(&mut self, service: &ConfiguredService) -> Option<String> {
        let command = service.version_command.as_ref()?;
        if let Some((read, version)) = self.versions.get(command)
            && read.elapsed() < VERSION_TTL
        {
            return version.clone();
        }
        let version = command_output(command)
            .and_then(|output| version_from_output(&output, service.version_word));
        self.versions
            .insert(command.clone(), (Instant::now(), version.clone()));
        version
    }
}
impl ServiceInfoSource for ServiceReader {
    fn read(&mut self) -> Result<Vec<ServiceEntry>, PortError> {
        let services = self.load()?;
        let states = read_states(&services);
        Ok(services
            .into_iter()
            .zip(states)
            .map(|(service, state)| ServiceEntry {
                version: self.version(&service),
                group: service.group,
                name: service.name,
                state,
                actions: if self.report_actions {
                    service.actions
                } else {
                    Vec::new()
                },
                web: service.web,
            })
            .collect())
    }
}
fn read_states(services: &[ConfiguredService]) -> Vec<ServiceState> {
    if services.is_empty() {
        return Vec::new();
    }
    let mut arguments = vec![
        "show".to_owned(),
        "--property=LoadState".into(),
        "--property=ActiveState".into(),
    ];
    arguments.extend(services.iter().map(|service| service.unit.clone()));
    let mut command = vec!["systemctl".to_owned()];
    command.extend(arguments);
    match command_output(&command) {
        Some(output) => parse_states(&output, services.len()),
        None => vec![ServiceState::Unknown; services.len()],
    }
}

/// Queues actions for the root helper (`run_requests`): one small file per request.
pub struct ServiceControlQueue {
    configuration: PathBuf,
    requests: PathBuf,
    counter: u64,
}
impl ServiceControlQueue {
    pub fn new(configuration: &Path, requests: &Path) -> Self {
        Self {
            configuration: configuration.to_owned(),
            requests: requests.to_owned(),
            counter: 0,
        }
    }
    /// The feature is only offered where the installer enabled the helper's path unit.
    pub fn installed(unit_directory: &Path) -> bool {
        REQUEST_UNITS.iter().all(|unit| {
            unit_directory
                .join("multi-user.target.wants")
                .join(unit)
                .exists()
        })
    }
}
fn action_word(action: ServiceAction) -> &'static str {
    match action {
        ServiceAction::Start => "start",
        ServiceAction::Stop => "stop",
        ServiceAction::Restart => "restart",
    }
}
impl ServiceControl for ServiceControlQueue {
    fn perform(&mut self, name: &str, action: ServiceAction) -> Result<(), ServiceControlError> {
        let (services, _) =
            load_directory(&self.configuration).map_err(|_| ServiceControlError::Unavailable)?;
        let service = services
            .iter()
            .find(|service| service.name == name)
            .ok_or(ServiceControlError::Unknown)?;
        if !service.actions.contains(&action) {
            return Err(ServiceControlError::NotAllowed);
        }
        let pending = fs::read_dir(&self.requests)
            .map_err(|_| ServiceControlError::Unavailable)?
            .count();
        if pending >= MAX_PENDING_REQUESTS {
            return Err(ServiceControlError::Busy);
        }
        self.counter += 1;
        let file = self.requests.join(format!("{:020}", self.counter));
        let content = format!("{} {}\n", action_word(action), service.unit);
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file)
            .and_then(|mut file| std::io::Write::write_all(&mut file, content.as_bytes()))
            .map_err(|_| ServiceControlError::Unavailable)
    }
}

/// Root side: performs and removes the queued requests. The files come from an unprivileged
/// user, so each one is checked against the root-owned configuration before `systemctl` runs.
pub fn run_requests(requests: &Path, configuration: &Path) -> std::io::Result<()> {
    let (services, _) = load_directory(configuration)
        .map_err(|_| std::io::Error::other("service configuration unavailable"))?;
    let mut files = fs::read_dir(requests)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    files.sort();
    for file in files {
        let request = read_request(&file);
        // Removed first, so a failing request cannot trigger the helper again and again.
        fs::remove_file(&file)?;
        let Some((action, unit)) = request.and_then(|text| parse_request(&text, &services)) else {
            eprintln!("Service request rejected: {}", file.display());
            continue;
        };
        let status = Command::new("systemctl")
            .args(["--no-block", action_word(action), "--", &unit])
            .stdin(Stdio::null())
            .status();
        match status {
            Ok(status) if status.success() => (),
            Ok(status) => eprintln!("systemctl {} {unit} failed: {status}", action_word(action)),
            Err(error) => eprintln!("systemctl could not run: {error}"),
        }
    }
    Ok(())
}
fn read_request(file: &Path) -> Option<String> {
    let metadata = fs::symlink_metadata(file).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_REQUEST_BYTES {
        return None;
    }
    fs::read_to_string(file).ok()
}
fn parse_request(text: &str, services: &[ConfiguredService]) -> Option<(ServiceAction, String)> {
    let (word, unit) = text.trim_end_matches('\n').split_once(' ')?;
    let action = [
        ServiceAction::Start,
        ServiceAction::Stop,
        ServiceAction::Restart,
    ]
    .into_iter()
    .find(|action| action_word(*action) == word)?;
    services
        .iter()
        .any(|service| service.unit == unit && service.actions.contains(&action))
        .then(|| (action, unit.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_file_gets_defaults_and_full_file_keeps_all_fields() {
        let minimal = parse_service("name = \"Raspotify\"\nunit = \"raspotify.service\"").unwrap();
        assert_eq!(minimal.group, DEFAULT_GROUP);
        assert!(minimal.web.is_none() && minimal.version_command.is_none());
        let full = parse_service(
            r#"
            name = "Mopidy"
            group = "Musik"
            unit = "mopidy.service"
            version_command = ["mopidy", "--version"]
            version_word = 2
            [web]
            label = "Iris"
            scheme = "https"
            port = 6680
            path = "/iris/"
            "#,
        )
        .unwrap();
        assert_eq!(full.group, "Musik");
        assert_eq!(full.version_word, Some(2));
        let web = full.web.unwrap();
        assert_eq!(
            (web.scheme, web.port, web.path.as_str()),
            (WebScheme::Https, 6680, "/iris/")
        );
        assert_eq!(web.label.as_deref(), Some("Iris"));
    }

    #[test]
    fn invalid_files_are_rejected() {
        let file = |extra: &str| format!("name = \"A\"\nunit = \"a.service\"\n{extra}");
        for invalid in [
            "name = \"A\"".to_owned(),
            file("actions = [\"reload\"]"),
            file("actions = [\"stop\", \"stop\"]"),
            "name = \"A\"\nunit = \"bluetooth.service\"\nactions = [\"stop\"]".into(),
            "name = \"A\"\nunit = \"zorvinkeld.service\"\nactions = [\"restart\"]".into(),
            "name = \"A\"\nunit = \"systemd-logind.service\"\nactions = [\"restart\"]".into(),
            file("group = \"\""),
            file("version_command = []"),
            file("version_word = 0"),
            file("[web]\nport = 0"),
            file("[web]\nport = 80\nscheme = \"ftp\""),
            file("[web]\nport = 80\npath = \"no-slash\""),
            file("[web]\nport = 80\npath = \"/a b\""),
            file("[web]\nport = 70000"),
            "name = \"A\"\nunit = \"-a.service\"".into(),
            "name = \"A\"\nunit = \"a.socket\"".into(),
            "name = \"A\"\nunit = \"a b.service\"".into(),
            format!(
                "name = \"{}\"\nunit = \"a.service\"",
                "x".repeat(MAX_NAME + 1)
            ),
        ] {
            assert!(parse_service(&invalid).is_err(), "accepted: {invalid}");
        }
    }

    #[test]
    fn states_follow_request_order_and_fall_back_to_unknown() {
        let output = "LoadState=loaded\nActiveState=active\n\nLoadState=loaded\nActiveState=failed\n\nLoadState=not-found\nActiveState=inactive\n\nLoadState=loaded\nActiveState=activating\n";
        assert_eq!(
            parse_states(output, 4),
            [
                ServiceState::Active,
                ServiceState::Failed,
                ServiceState::Unknown,
                ServiceState::Activating
            ]
        );
        assert_eq!(parse_states(output, 3), vec![ServiceState::Unknown; 3]);
        assert_eq!(parse_states("", 1), vec![ServiceState::Unknown]);
    }

    #[test]
    fn version_is_taken_from_the_first_line() {
        assert_eq!(
            version_from_output("\nlibrespot 0.4.2 abc built\nmore\n", Some(2)).as_deref(),
            Some("0.4.2")
        );
        assert_eq!(
            version_from_output("3.4.2\n", None).as_deref(),
            Some("3.4.2")
        );
        assert_eq!(version_from_output("one\n", Some(2)), None);
        assert_eq!(version_from_output("   \n", None), None);
        assert_eq!(
            version_from_output(&"x".repeat(MAX_VERSION + 1), None),
            None
        );
    }

    #[test]
    fn reader_lists_valid_files_in_order_and_skips_broken_ones() {
        let directory =
            std::env::temp_dir().join(format!("zorvinkel-services-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("b.toml"),
            "name = \"B\"\nunit = \"b.service\"",
        )
        .unwrap();
        fs::write(
            directory.join("a.toml"),
            "name = \"A\"\ngroup = \"G\"\nunit = \"a.service\"",
        )
        .unwrap();
        fs::write(directory.join("broken.toml"), "name = ").unwrap();
        fs::write(directory.join("ignored.txt"), "name = \"X\"").unwrap();
        let mut reader = ServiceReader::new(&directory, true);
        let services = reader.load().unwrap();
        let _ = fs::remove_dir_all(&directory);
        assert_eq!(
            services
                .iter()
                .map(|service| service.name.as_str())
                .collect::<Vec<_>>(),
            ["A", "B"]
        );
        assert_eq!(reader.reported_problems.len(), 1);
    }

    fn directory(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("zorvinkel-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }
    fn configured(directory: &Path) {
        fs::write(
            directory.join("a.toml"),
            "name = \"A\"\nunit = \"a.service\"\nactions = [\"start\", \"restart\"]",
        )
        .unwrap();
        fs::write(
            directory.join("b.toml"),
            "name = \"B\"\nunit = \"b.service\"",
        )
        .unwrap();
        // Requests name a service, so a repeated name leaves the later file out.
        fs::write(
            directory.join("c.toml"),
            "name = \"A\"\nunit = \"c.service\"\nactions = [\"stop\"]",
        )
        .unwrap();
    }

    #[test]
    fn display_only_services_report_no_actions_and_duplicates_are_left_out() {
        let configuration = directory("actions-config");
        configured(&configuration);
        let (services, problems) = load_directory(&configuration).unwrap();
        let _ = fs::remove_dir_all(&configuration);
        assert_eq!(services.len(), 2);
        assert_eq!(
            services[0].actions,
            [ServiceAction::Start, ServiceAction::Restart]
        );
        assert!(services[1].actions.is_empty());
        assert_eq!(problems.len(), 1);
    }

    #[test]
    fn queue_accepts_only_configured_actions_and_limits_pending_requests() {
        let (configuration, requests) = (directory("queue-config"), directory("queue-requests"));
        configured(&configuration);
        let mut queue = ServiceControlQueue::new(&configuration, &requests);
        assert_eq!(
            queue.perform("Nobody", ServiceAction::Start),
            Err(ServiceControlError::Unknown)
        );
        assert_eq!(
            queue.perform("A", ServiceAction::Stop),
            Err(ServiceControlError::NotAllowed)
        );
        assert_eq!(
            queue.perform("B", ServiceAction::Start),
            Err(ServiceControlError::NotAllowed)
        );
        assert_eq!(fs::read_dir(&requests).unwrap().count(), 0);
        assert_eq!(queue.perform("A", ServiceAction::Restart), Ok(()));
        let file = fs::read_dir(&requests)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(fs::read_to_string(file).unwrap(), "restart a.service\n");
        for _ in 1..MAX_PENDING_REQUESTS {
            assert_eq!(queue.perform("A", ServiceAction::Start), Ok(()));
        }
        assert_eq!(
            queue.perform("A", ServiceAction::Start),
            Err(ServiceControlError::Busy)
        );
        let _ = fs::remove_dir_all(&configuration);
        let _ = fs::remove_dir_all(&requests);
    }

    #[test]
    fn helper_only_acts_on_requests_the_configuration_allows() {
        let configuration = directory("helper-config");
        configured(&configuration);
        let (services, _) = load_directory(&configuration).unwrap();
        let _ = fs::remove_dir_all(&configuration);
        let accepted = |text: &str| parse_request(text, &services);
        assert_eq!(
            accepted("start a.service\n"),
            Some((ServiceAction::Start, "a.service".to_owned()))
        );
        for rejected in [
            "stop a.service\n",
            "start b.service\n",
            "start other.service\n",
            "reload a.service\n",
            "start  a.service",
            "start a.service extra",
            "start --now",
            "start",
            "",
        ] {
            assert_eq!(accepted(rejected), None, "{rejected:?}");
        }
    }

    #[test]
    fn helper_removes_every_request_including_rejected_and_oversized_ones() {
        let (configuration, requests) = (directory("run-config"), directory("run-requests"));
        configured(&configuration);
        fs::write(requests.join("1"), "stop a.service\n").unwrap();
        fs::write(
            requests.join("2"),
            "x".repeat(MAX_REQUEST_BYTES as usize + 1),
        )
        .unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hostname", requests.join("3")).unwrap();
        run_requests(&requests, &configuration).unwrap();
        let left = fs::read_dir(&requests).unwrap().count();
        let _ = fs::remove_dir_all(&configuration);
        let _ = fs::remove_dir_all(&requests);
        assert_eq!(left, 0);
    }

    #[test]
    fn missing_directory_is_not_configured() {
        assert!(!ServiceReader::new(Path::new("/nonexistent/zorvinkel"), false).configured());
    }
}
