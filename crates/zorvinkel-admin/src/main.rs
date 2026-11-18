use std::process::ExitCode;
fn main() -> ExitCode {
    #[cfg(unix)]
    {
        use std::{
            io::{Read, Write},
            os::unix::net::UnixStream,
            time::Duration,
        };
        let args: Vec<_> = std::env::args().skip(1).collect();
        let result = (|| {
            let persistence = args.len() == 3
                && args[1] == "wifi-persistence"
                && matches!(args[2].as_str(), "on" | "off" | "status");
            if !persistence
                && (!(2..=3).contains(&args.len())
                    || !matches!(
                        args[1].as_str(),
                        "public-key" | "grant" | "authorize" | "revoke"
                    )
                    || (args[1] == "public-key") != (args.len() == 2))
            {
                return Err("Usage: zorvinkel-admin socket public-key | socket grant|authorize|revoke public-key-hex | socket wifi-persistence on|off|status".into());
            }
            let mut stream = UnixStream::connect(&args[0])?;
            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
            stream.set_write_timeout(Some(Duration::from_secs(3)))?;
            stream.write_all(format!("{}\n", args[1..].join(" ")).as_bytes())?;
            let mut response = String::new();
            stream.take(128).read_to_string(&mut response)?;
            if response == "OK\n"
                || (persistence && matches!(response.as_str(), "on\n" | "off\n"))
                || (args[1] == "public-key"
                    && response.trim().len() == 64
                    && response.trim().bytes().all(|b| b.is_ascii_hexdigit()))
            {
                print!("{response}");
                Ok(())
            } else {
                Err("Administrator request rejected".into())
            }
        })();
        match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                let error: Box<dyn std::error::Error> = error;
                eprintln!("{error}");
                ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(unix))]
    {
        eprintln!("Local administration requires a Unix system");
        ExitCode::FAILURE
    }
}
