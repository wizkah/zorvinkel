use std::process::ExitCode;
use zorvinkel_adapters::simulation::SimulatedNetwork;
use zorvinkel_application::device::Device;

fn main() -> ExitCode {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    #[cfg(all(target_os = "linux", feature = "security", feature = "ble-server"))]
    if arguments.len() == 2 && arguments[0] == "--check-storage" {
        return match zorvinkel_adapters::linux::runtime::RuntimeState::open(std::path::Path::new(
            &arguments[1],
        )) {
            Ok(_) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Storage check failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    // Root helper of the service actions (zorvinkel-service.service); see packaging/linux.
    #[cfg(all(target_os = "linux", feature = "security", feature = "ble-server"))]
    if arguments.len() == 3 && arguments[0] == "--run-service-requests" {
        return match zorvinkel_adapters::linux::services::run_requests(
            std::path::Path::new(&arguments[1]),
            std::path::Path::new(&arguments[2]),
        ) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Service requests failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    #[cfg(all(unix, feature = "security"))]
    if arguments.len() == 2 && arguments[0] == "--initialize" {
        return match zorvinkel_adapters::security::storage::initialize_device(std::path::Path::new(
            &arguments[1],
        )) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Initialization failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    #[cfg(all(target_os = "linux", feature = "security", feature = "ble-server"))]
    if arguments.len() == 4 && arguments[0] == "--serve" {
        let result = tokio::runtime::Runtime::new().map_err(|e| e.to_string()).and_then(|runtime| runtime.block_on(zorvinkel_adapters::linux::runtime::run(
            std::path::Path::new(&arguments[1]), std::path::Path::new(&arguments[2]), arguments[3].clone(), async {
                match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                    Ok(mut terminate) => { tokio::select! { _ = tokio::signal::ctrl_c() => (), _ = terminate.recv() => () } },
                    Err(error) => eprintln!("Signal handler failed: {error}"),
                }
            }
        )).map_err(|e| e.to_string()));
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("Device service failed: {error}");
                ExitCode::FAILURE
            }
        };
    }
    #[cfg(all(feature = "ble-server", target_os = "linux"))]
    if arguments == ["--ble-probe"] {
        return run_ble_probe();
    }
    #[cfg(all(feature = "ble-server", target_os = "linux"))]
    if arguments == ["--ble-bench"] {
        return run_ble_bench();
    }
    if arguments != ["--simulate"] {
        eprintln!(
            "Usage: zorvinkeld --simulate | --ble-probe | --ble-bench | --initialize directory | --check-storage directory | --serve directory admin-socket interface (native service requires Linux, security and ble-server features)"
        );
        return ExitCode::FAILURE;
    }
    match Device::new(SimulatedNetwork::default()).network_status() {
        Ok(status) => {
            println!("Simulation: network status: {status:?}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Simulation failed: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(all(feature = "ble-server", target_os = "linux"))]
fn run_ble_probe() -> ExitCode {
    let result = (|| {
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(zorvinkel_adapters::ble::server::serve_probe(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                eprintln!("Shutdown signal failed: {error}");
            }
        }))
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("BLE probe server failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Radio measurement service for the terminal tunnel limits (ADR-031); carries no application data.
#[cfg(all(feature = "ble-server", target_os = "linux"))]
fn run_ble_bench() -> ExitCode {
    let result = (|| {
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(zorvinkel_adapters::ble::bench::serve_bench(async {
            if let Err(error) = tokio::signal::ctrl_c().await {
                eprintln!("Shutdown signal failed: {error}");
            }
        }))
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("BLE bench server failed: {error}");
            ExitCode::FAILURE
        }
    }
}
