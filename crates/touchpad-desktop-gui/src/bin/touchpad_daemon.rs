//! Headless TouchPad Remote UDP daemon (`touchpad-daemon`).
//!
//! Runs without X11/Wayland GUI dependencies using `LinuxInputBackend` on Linux
//! and `WindowsInputBackend` on Windows.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use touchpad_server_core::{get_local_ip_addresses, InputBackend, ServerEvent, TouchpadServer};
use touchpad_storage::{default_user_desktop_config_path, ConfigStore, DesktopConfig};

static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, PartialEq, Eq)]
struct DaemonArgs {
    auto_approve: bool,
    port_override: Option<u16>,
    config_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliCommand {
    Run(DaemonArgs),
    Help,
}

fn parse_cli_args<I, S>(args: I) -> Result<CliCommand, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut parsed = DaemonArgs {
        auto_approve: false,
        port_override: None,
        config_path: None,
    };

    let mut iter = args.into_iter();
    while let Some(raw_arg) = iter.next() {
        let arg = raw_arg.as_ref();
        match arg {
            "-h" | "--help" => return Ok(CliCommand::Help),
            "--auto-approve" => parsed.auto_approve = true,
            "--port" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "O parâmetro --port exige um número entre 1 e 65535.".to_string())?;
                let port: u16 = value
                    .as_ref()
                    .trim()
                    .parse()
                    .ok()
                    .filter(|&p: &u16| p > 0)
                    .ok_or_else(|| {
                        format!(
                            "Porta UDP inválida '{}': informe um valor entre 1 e 65535.",
                            value.as_ref()
                        )
                    })?;
                parsed.port_override = Some(port);
            }
            "--config" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "O parâmetro --config exige um caminho de arquivo.".to_string())?;
                let trimmed = value.as_ref().trim();
                if trimmed.is_empty() {
                    return Err("O caminho informado em --config não pode ser vazio.".to_string());
                }
                parsed.config_path = Some(PathBuf::from(trimmed));
            }
            unknown => {
                return Err(format!("Argumento desconhecido: {unknown}"));
            }
        }
    }

    Ok(CliCommand::Run(parsed))
}

fn print_usage() {
    println!(
        "Uso: touchpad-daemon [--auto-approve] [--port <1-65535>] [--config <caminho>]\n\
         \n\
         Opções:\n\
           --auto-approve      Aprova automaticamente novos pedidos de pareamento\n\
           --port <u16>        Sobrescreve a porta UDP configurada (padrão: config.json / 45454)\n\
           --config <caminho>  Usa um arquivo config.json alternativo\n\
           -h, --help          Exibe esta ajuda"
    );
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

#[cfg(windows)]
fn install_signal_handlers() {
    extern "system" {
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
    }

    unsafe extern "system" fn console_ctrl_handler(_ctrl_type: u32) -> i32 {
        SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
        1
    }

    // SAFETY: `console_ctrl_handler` follows the Win32 `PHANDLER_ROUTINE` ABI and only flips an AtomicBool.
    let ok = unsafe { SetConsoleCtrlHandler(Some(console_ctrl_handler), 1) };
    if ok == 0 {
        tracing::warn!("Falha ao registrar handler de Ctrl+C no console do Windows");
    }
}

#[cfg(unix)]
fn install_signal_handlers() {
    const SIGINT: i32 = 2;
    const SIGTERM: i32 = 15;

    extern "C" {
        fn signal(sig: i32, handler: extern "C" fn(i32)) -> usize;
    }

    extern "C" fn unix_signal_handler(_sig: i32) {
        SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
    }

    // SAFETY: `unix_signal_handler` is async-signal-safe (only stores to an AtomicBool).
    unsafe {
        signal(SIGINT, unix_signal_handler);
        signal(SIGTERM, unix_signal_handler);
    }
}

#[cfg(not(any(windows, unix)))]
fn install_signal_handlers() {}

fn build_input_backend() -> Result<Arc<dyn InputBackend>, String> {
    #[cfg(windows)]
    {
        Ok(Arc::new(touchpad_input_win::WindowsInputBackend::new()))
    }

    #[cfg(target_os = "linux")]
    {
        match touchpad_input_linux::LinuxInputBackend::new() {
            Ok(backend) => {
                let status = backend.status();
                tracing::info!(
                    session = ?status.session,
                    injection = ?status.injection,
                    "Backend de entrada Linux inicializado"
                );
                if let Some(hint) = status.hint {
                    tracing::warn!("{hint}");
                }
                Ok(Arc::new(backend))
            }
            Err(err) => {
                tracing::error!(
                    session = ?err.status.session,
                    injection = ?err.status.injection,
                    "{err}"
                );
                Err(err.to_string())
            }
        }
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Ok(Arc::new(touchpad_server_core::DryRunBackend::default()))
    }
}

fn run_daemon(args: DaemonArgs) -> i32 {
    init_logging();
    install_signal_handlers();

    let config_path = args
        .config_path
        .unwrap_or_else(default_user_desktop_config_path);
    let store = match ConfigStore::<DesktopConfig>::new(config_path.clone()) {
        Ok(store) => Arc::new(store),
        Err(err) => {
            tracing::error!(path = %config_path.display(), %err, "Falha ao carregar configuração");
            return 1;
        }
    };

    let backend = match build_input_backend() {
        Ok(backend) => backend,
        Err(err) => {
            eprintln!("Erro ao inicializar entrada do sistema: {err}");
            return 1;
        }
    };

    let (auto_approve_tx, auto_approve_rx) = mpsc::channel::<IpAddr>();
    let auto_approve = args.auto_approve;

    let mut server = match args.port_override {
        Some(port) => TouchpadServer::with_port(backend, Arc::clone(&store), port),
        None => TouchpadServer::new(backend, Arc::clone(&store)),
    };

    server = server.with_event_callback(Arc::new(move |event| match event {
        ServerEvent::StateChanged { running, port } => {
            tracing::info!(running, port, "Estado do servidor alterado");
        }
        ServerEvent::DiscoveryRequested {
            client_name,
            client_ip,
            responded_ip,
        } => {
            tracing::info!(%client_name, %client_ip, %responded_ip, "Descoberta respondida");
        }
        ServerEvent::PingReceived { client_ip } => {
            tracing::debug!(%client_ip, "Ping recebido");
        }
        ServerEvent::PairingRequested(client) => {
            tracing::info!(
                client_id = %client.client_id,
                client_name = %client.client_name,
                ip = %client.ip_address,
                auto_approve,
                "Solicitação de pareamento recebida"
            );
            if auto_approve {
                let _ = auto_approve_tx.send(client.ip_address);
            }
        }
        ServerEvent::PairingStateChanged => {
            tracing::info!("Lista de pareamento atualizada");
        }
        ServerEvent::PacketReceived {
            client_ip,
            total_packets,
            ..
        } => {
            tracing::trace!(%client_ip, total_packets, "Pacote de controle processado");
        }
        ServerEvent::Error(message) => {
            tracing::error!(%message, "Erro no servidor TouchPad Remote");
        }
    }));

    if let Err(err) = server.start() {
        tracing::error!(%err, "Não foi possível iniciar o servidor UDP");
        return 1;
    }

    let snapshot = server.snapshot();
    let ips = get_local_ip_addresses();
    tracing::info!(
        computer = %snapshot.config.computer_name,
        port = snapshot.actual_port,
        config_path = %config_path.display(),
        saved_devices = snapshot.config.allowed_devices.len(),
        require_pairing_approval = snapshot.config.require_pairing_approval,
        auto_approve,
        local_ips = ?ips,
        "touchpad-daemon iniciado. Pressione Ctrl+C para encerrar."
    );

    while !SHUTDOWN_REQUESTED.load(Ordering::Acquire) && server.is_running() {
        while let Ok(ip) = auto_approve_rx.try_recv() {
            if let Err(err) = server.approve_pairing(ip) {
                tracing::warn!(%ip, %err, "Falha no pareamento automático");
            } else {
                tracing::info!(%ip, "Dispositivo aprovado automaticamente (--auto-approve)");
            }
        }
        thread::sleep(Duration::from_millis(50));
    }

    tracing::info!("Encerrando touchpad-daemon, liberando botões e salvando configuração...");
    server.stop();
    let final_config = store.get();
    if let Err(err) = store.save(final_config) {
        tracing::warn!(%err, "Falha ao salvar configuração durante o encerramento");
    }
    tracing::info!("touchpad-daemon encerrado com segurança.");
    0
}

fn main() {
    match parse_cli_args(std::env::args().skip(1)) {
        Ok(CliCommand::Help) => {
            print_usage();
            std::process::exit(0);
        }
        Ok(CliCommand::Run(args)) => {
            std::process::exit(run_daemon(args));
        }
        Err(err) => {
            eprintln!("{err}\n");
            print_usage();
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_cli_args() {
        let parsed = parse_cli_args::<[&str; 0], &str>([]).unwrap();
        assert_eq!(
            parsed,
            CliCommand::Run(DaemonArgs {
                auto_approve: false,
                port_override: None,
                config_path: None,
            })
        );
    }

    #[test]
    fn parses_auto_approve_port_and_config() {
        let parsed = parse_cli_args([
            "--auto-approve",
            "--port",
            "46000",
            "--config",
            "/tmp/touchpad.json",
        ])
        .unwrap();
        assert_eq!(
            parsed,
            CliCommand::Run(DaemonArgs {
                auto_approve: true,
                port_override: Some(46000),
                config_path: Some(PathBuf::from("/tmp/touchpad.json")),
            })
        );
    }

    #[test]
    fn parses_help_and_rejects_invalid_flags() {
        assert_eq!(parse_cli_args(["--help"]).unwrap(), CliCommand::Help);
        assert_eq!(parse_cli_args(["-h"]).unwrap(), CliCommand::Help);
        assert!(parse_cli_args(["--port", "0"]).is_err());
        assert!(parse_cli_args(["--port"]).is_err());
        assert!(parse_cli_args(["--port", "abc"]).is_err());
        assert!(parse_cli_args(["--config", ""]).is_err());
        assert!(parse_cli_args(["--unknown"]).is_err());
    }
}
