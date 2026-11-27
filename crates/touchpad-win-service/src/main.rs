//! Entry point for `touchpad-win-service.exe`.

#[cfg(windows)]
mod desktop_agent;
#[cfg(windows)]
mod install;
#[cfg(windows)]
mod service_runtime;
#[cfg(windows)]
mod session_monitor;

fn main() {
    #[cfg(windows)]
    {
        init_logging();
        std::process::exit(run_cli());
    }

    #[cfg(not(windows))]
    {
        eprintln!("touchpad-win-service is only supported on Windows.");
        std::process::exit(1);
    }
}

#[cfg(windows)]
fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

#[cfg(windows)]
fn run_cli() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() {
        return match service_runtime::run_service_dispatcher() {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("{err}");
                1
            }
        };
    }

    match args[0].as_str() {
        "--install" => {
            let mut user_config = None;
            let mut i = 1;
            while i < args.len() {
                if args[i] == "--user-config" {
                    let Some(path) = args.get(i + 1) else {
                        eprintln!("--user-config requires a file path");
                        return 2;
                    };
                    user_config = Some(std::path::PathBuf::from(path));
                    i += 2;
                } else {
                    eprintln!("Unknown argument for --install: {}", args[i]);
                    print_usage();
                    return 2;
                }
            }
            match install::install_service(user_config) {
                Ok(()) => 0,
                Err(err) => {
                    eprintln!("Install failed: {err}");
                    1
                }
            }
        }
        "--uninstall" if args.len() == 1 => match install::uninstall_service() {
            Ok(()) => 0,
            Err(err) => {
                eprintln!("Uninstall failed: {err}");
                1
            }
        },
        "--status" if args.len() == 1 => install::run_status(),
        "--desktop-agent" if args.len() == 1 => match desktop_agent::run_desktop_agent() {
            Ok(()) => 0,
            Err(err) => {
                tracing::error!(%err, "Desktop agent exited with error");
                eprintln!("Desktop agent failed: {err}");
                1
            }
        },
        "--help" | "-h" => {
            print_usage();
            0
        }
        _ => {
            eprintln!("Invalid arguments: {}", args.join(" "));
            print_usage();
            2
        }
    }
}

#[cfg(windows)]
fn print_usage() {
    println!(
        "Usage: touchpad-win-service.exe [--install [--user-config <path>] | --uninstall | --status | --desktop-agent]"
    );
}
