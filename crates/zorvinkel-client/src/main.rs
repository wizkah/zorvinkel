// A double click must not open a console window; command-line output is not shown in this build.
#![cfg_attr(
    all(windows, feature = "gui", feature = "security", feature = "ble-client"),
    windows_subsystem = "windows"
)]
#[cfg(feature = "gui")]
mod gui;
#[cfg(feature = "security")]
mod secure_demo;
use std::process::ExitCode;
use zorvinkel_adapters::simulation::{SimulatedDiscovery, SimulatedNetwork, SimulatedStatusSource};
use zorvinkel_application::client::Client;

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    #[cfg(feature = "gui")]
    if arguments.len() == 2 && arguments[0] == "--export-app-icon" {
        return match gui::export_app_icon(std::path::Path::new(&arguments[1])) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("App icon export failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    #[cfg(all(any(unix, windows), feature = "security", feature = "ble-client"))]
    if arguments.len() == 2 && arguments[0] == "--client-key" {
        return match zorvinkel_adapters::ble::setup::NativeSetup::open(std::path::Path::new(
            &arguments[1],
        )) {
            Ok(backend) => {
                println!(
                    "{}",
                    backend
                        .public_key()
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect::<String>()
                );
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("Client storage failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    #[cfg(all(any(unix, windows), feature = "security", feature = "ble-client"))]
    if arguments.len() == 4 && arguments[0] == "--trust-device" {
        use zorvinkel_application::trust::configure_device;
        let result = (|| {
            let root = std::path::Path::new(&arguments[1]);
            let _backend = zorvinkel_adapters::ble::setup::NativeSetup::open(root)?;
            let hex = &arguments[3];
            if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("expected 32-byte device public key in hexadecimal".into());
            }
            let bytes = (0..64)
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
                .collect::<Result<Vec<_>, _>>()?;
            let key =
                zorvinkel_core::trust::DeviceKey::new(bytes).map_err(|_| "invalid device key")?;
            let mut trust = zorvinkel_adapters::persistence::catalog::select(root, &arguments[2])
                .map_err(|_| "device binding unavailable")?;
            configure_device(&mut trust, key).map_err(|_| "device binding rejected")?;
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        })();
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Device binding failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    #[cfg(all(
        any(unix, windows),
        feature = "gui",
        feature = "security",
        feature = "ble-client"
    ))]
    if arguments.is_empty()
        || arguments == ["--device-gui"]
        || (arguments.len() == 2 && arguments[0] == "--device-gui")
    {
        let directory = if let Some(directory) = arguments.get(1) {
            std::path::PathBuf::from(directory)
        } else {
            let Some(parent) = default_data_parent() else {
                eprintln!("User directory unavailable");
                return ExitCode::FAILURE;
            };
            if let Err(error) = std::fs::create_dir_all(&parent) {
                eprintln!("Client directory unavailable: {error}");
                return ExitCode::FAILURE;
            }
            parent.join("Zorvinkel")
        };
        return match zorvinkel_adapters::ble::setup::NativeSetup::open(&directory) {
            Ok(backend) => match gui::run_backend(backend, &directory) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("GUI failed: {error}");
                    ExitCode::FAILURE
                }
            },
            Err(error) => {
                eprintln!("Client storage failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    #[cfg(all(
        any(unix, windows),
        feature = "security",
        feature = "ble-client",
        feature = "gui"
    ))]
    if arguments.len() == 5 && arguments[0] == "--ble-ssh-test" {
        return ble_ssh_test(&arguments[1..]);
    }
    #[cfg(all(any(unix, windows), feature = "security", feature = "ble-client"))]
    if arguments.len() == 3 && arguments[0] == "--ble-tunnel-test" {
        return ble_tunnel_test(&arguments[1], &arguments[2]);
    }
    #[cfg(all(any(unix, windows), feature = "security", feature = "ble-client"))]
    if arguments.len() >= 2 && arguments[0] == "--device-status" {
        return native_status(&arguments[1..]);
    }
    #[cfg(feature = "security")]
    if arguments == ["--secure-simulate"] {
        return match secure_demo::run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Secure simulation failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    #[cfg(feature = "gui")]
    if arguments.first().map(String::as_str) == Some("--simulate-gui") {
        let Some(count) = simulated_device_count(&arguments[1..]) else {
            eprintln!("Usage: zorvinkel-client --simulate-gui [device-count 1-100]");
            return ExitCode::FAILURE;
        };
        return match gui::run(count) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("GUI failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    #[cfg(feature = "ble-client")]
    if arguments.first().map(String::as_str) == Some("--ble-probe") {
        return run_ble_probe(&arguments[1..]);
    }
    #[cfg(feature = "ble-client")]
    if arguments.first().map(String::as_str) == Some("--ble-bench") {
        return run_ble_bench(&arguments[1..]);
    }
    if arguments != ["--simulate"] {
        eprintln!(
            "Usage: zorvinkel-client --simulate | --secure-simulate | --simulate-gui [device-count] | --ble-probe [handle] | --ble-bench [handle] | --ble-tunnel-test directory handle | --device-status directory [handle] | --device-gui directory | --client-key directory | --trust-device directory handle key-hex (native commands require matching features; persistent native storage requires Unix)"
        );
        return ExitCode::FAILURE;
    }
    match simulate() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Simulation failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Per-user data directory: `%LOCALAPPDATA%` on Windows, Application Support on macOS, XDG data home on Linux.
#[cfg(all(
    any(unix, windows),
    feature = "gui",
    feature = "security",
    feature = "ble-client"
))]
fn default_data_parent() -> Option<std::path::PathBuf> {
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from);
    }
    let home_directory = std::path::PathBuf::from(std::env::var_os("HOME")?);
    Some(if cfg!(target_os = "macos") {
        home_directory.join("Library/Application Support")
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home_directory.join(".local/share"))
    })
}

/// Defaults to three devices; larger counts show the GUI under load.
#[cfg(feature = "gui")]
fn simulated_device_count(arguments: &[String]) -> Option<usize> {
    match arguments {
        [] => Some(3),
        [count] => count.parse().ok().filter(|count| (1..=100).contains(count)),
        _ => None,
    }
}

fn simulate() -> Result<(), zorvinkel_application::PortError> {
    let mut client = Client::new(
        SimulatedDiscovery,
        SimulatedStatusSource::new(SimulatedNetwork::default()),
    );
    for device in client.discover()? {
        let status = client.read_status(&device)?;
        println!("Simulation: {}: {status:?}", device.display_name);
    }
    Ok(())
}

#[cfg(feature = "ble-client")]
fn run_ble_probe(arguments: &[String]) -> ExitCode {
    if arguments.len() > 1 {
        eprintln!("Usage: zorvinkel-client --ble-probe [device-handle]");
        return ExitCode::FAILURE;
    }
    let result = (|| {
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(async {
            let mut central = zorvinkel_adapters::ble::client::ProbeCentral::open().await?;
            let devices = central.discover().await?;
            for device in &devices {
                println!("{}: {}", device.handle, device.display_name);
            }
            if let Some(handle) = arguments.first() {
                // Two independently established connections exercise the reconnect path.
                for round in 1..=2 {
                    central.probe(handle).await?;
                    println!("BLE probe round {round}: passed");
                }
            }
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        })
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("BLE probe failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(all(any(unix, windows), feature = "security", feature = "ble-client"))]
fn native_status(arguments: &[String]) -> ExitCode {
    use zorvinkel_application::setup::{SetupBackend, SetupRequest};
    let result = (|| {
        if arguments.len() > 2 {
            return Err("Usage: --device-status storage-directory [handle]".into());
        }
        let mut backend =
            zorvinkel_adapters::ble::setup::NativeSetup::open(std::path::Path::new(&arguments[0]))?;
        let key: String = backend
            .public_key()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        println!("Client public key: {key}");
        let update = backend
            .handle(SetupRequest::Discover)
            .map_err(|_| "discovery failed")?;
        if update.error.is_some() {
            return Err("discovery failed".into());
        }
        for device in update.snapshot.devices {
            println!("{}: {}", device.handle, device.display_name);
        }
        if let Some(handle) = arguments.get(1) {
            for _ in 0..2 {
                let update = backend
                    .handle(SetupRequest::Connect(handle.clone()))
                    .map_err(|_| "connection failed")?;
                if update.error.is_some() {
                    return Err("device request failed".into());
                }
                println!("Authenticated status: {:?}", update.snapshot.status);
                backend
                    .handle(SetupRequest::Disconnect)
                    .map_err(|_| "disconnect failed")?;
            }
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Device status failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Opens a terminal tunnel to a device and reads the start of the SSH exchange (ADR-031).
#[cfg(all(any(unix, windows), feature = "security", feature = "ble-client"))]
fn ble_tunnel_test(directory: &str, handle: &str) -> ExitCode {
    use zorvinkel_application::setup::{SetupBackend, SetupRequest};
    let result = (|| {
        let mut backend =
            zorvinkel_adapters::ble::setup::NativeSetup::open(std::path::Path::new(directory))?;
        backend.force_bluetooth();
        backend
            .handle(SetupRequest::Discover)
            .map_err(|_| "discovery failed")?;
        let update = backend
            .handle(SetupRequest::Connect(handle.to_owned()))
            .map_err(|_| "connection failed")?;
        if update.error.is_some() {
            return Err("device request failed".into());
        }
        println!("{}", backend.probe_tunnel()?);
        backend
            .handle(SetupRequest::Disconnect)
            .map_err(|_| "disconnect failed")?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Tunnel test failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Radio measurement against `zorvinkeld --ble-bench` (ADR-031): sizes, latency, throughput.
#[cfg(feature = "ble-client")]
fn run_ble_bench(arguments: &[String]) -> ExitCode {
    if arguments.len() > 1 {
        eprintln!("Usage: zorvinkel-client --ble-bench [device-handle]");
        return ExitCode::FAILURE;
    }
    let result = (|| {
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(zorvinkel_adapters::ble::bench::run_bench(
            arguments.first().map(String::as_str),
        ))
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("BLE bench failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Real SSH session through the Bluetooth tunnel (ADR-031): key login, `tmux`, and the delay of
/// keystroke echoes. Accepts any host key; it is a measurement tool, not a terminal.
#[cfg(all(
    any(unix, windows),
    feature = "security",
    feature = "ble-client",
    feature = "gui"
))]
fn ble_ssh_test(arguments: &[String]) -> ExitCode {
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    use zorvinkel_application::setup::{SetupBackend, SetupRequest};
    use zorvinkel_terminal::{
        ByteStream, ConnectError, ConnectionStatus, Credentials, Dialer, Dialing, ResumableEvent,
        ResumableTerminal, RetryPolicy, SshConnector, TerminalSize,
    };
    struct Once(std::sync::Mutex<Option<tokio::io::DuplexStream>>);
    impl Dialer for Once {
        fn dial(&self) -> Dialing<'_> {
            let stream = self.0.lock().ok().and_then(|mut slot| slot.take());
            Box::pin(async move {
                stream
                    .map(|stream| Box::new(stream) as Box<dyn ByteStream>)
                    .ok_or_else(|| ConnectError::Refused("tunnel already used".into()))
            })
        }
    }
    let result = (|| {
        let [directory, handle, user, key_file] = arguments else {
            return Err("Usage: --ble-ssh-test directory handle user key-file".into());
        };
        let mut backend =
            zorvinkel_adapters::ble::setup::NativeSetup::open(std::path::Path::new(directory))?;
        backend.force_bluetooth();
        backend
            .handle(SetupRequest::Discover)
            .map_err(|_| "discovery failed")?;
        let update = backend
            .handle(SetupRequest::Connect(handle.clone()))
            .map_err(|_| "connection failed")?;
        if update.error.is_some() {
            return Err("device request failed".into());
        }
        let stream = backend.open_tunnel()?;
        let key = std::fs::read_to_string(key_file)?;
        let credentials = Credentials::private_key(user, &key, None).map_err(|_| "key rejected")?;
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(async {
            let connector = SshConnector::with_dialer(
                Arc::new(Once(std::sync::Mutex::new(Some(stream)))),
                credentials,
                Arc::new(|fingerprint: &str| {
                    println!("Host key {fingerprint}");
                    true
                }),
            );
            let started = Instant::now();
            let mut terminal = ResumableTerminal::start(
                connector,
                TerminalSize::new(100, 30).expect("size"),
                RetryPolicy::default(),
                || {},
            );
            let wait = Duration::from_secs(30);
            loop {
                match tokio::time::timeout(wait, terminal.next_event()).await {
                    Ok(Some(ResumableEvent::Status(ConnectionStatus::Connected))) => break,
                    Ok(Some(ResumableEvent::Status(ConnectionStatus::Failed(error)))) => {
                        return Err(format!("login failed: {error}").into());
                    }
                    Ok(Some(_)) => {}
                    _ => return Err("no connection within 30 s".into()),
                }
            }
            println!("SSH session open after {:?}", started.elapsed());
            // Let the shell and `tmux` settle, then time single keystrokes.
            let settle = tokio::time::sleep(Duration::from_secs(2));
            tokio::pin!(settle);
            loop {
                tokio::select! {
                    () = &mut settle => break,
                    _ = terminal.next_event() => {}
                }
            }
            let mut samples = Vec::new();
            for _ in 0..30 {
                let sent = Instant::now();
                terminal
                    .input(b"x".to_vec())
                    .await
                    .map_err(|_| "input failed")?;
                loop {
                    match tokio::time::timeout(Duration::from_secs(5), terminal.next_event()).await
                    {
                        Ok(Some(ResumableEvent::Output(_))) => break,
                        Ok(Some(_)) => {}
                        _ => return Err("no echo within 5 s".into()),
                    }
                }
                samples.push(sent.elapsed());
            }
            samples.sort();
            println!(
                "Keystroke echo: median {:?}, 95th percentile {:?}, maximum {:?} ({} samples)",
                samples[samples.len() / 2],
                samples[samples.len() * 95 / 100],
                samples[samples.len() - 1],
                samples.len()
            );
            terminal.input(b"\x15exit\r".to_vec()).await.ok();
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
        })?;
        backend
            .handle(SetupRequest::Disconnect)
            .map_err(|_| "disconnect failed")?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("SSH test failed: {error}");
            ExitCode::FAILURE
        }
    }
}
