//! Windows SCM installation, uninstallation, firewall setup, and `--status` reporting.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

use touchpad_protocol::DEFAULT_PORT;
use touchpad_storage::{
    default_user_desktop_config_path, migrate_user_config_to_program_data,
    windows_service_config_path, ConfigStore, DesktopConfig,
};
use touchpad_win_service::ipc_client::{
    is_service_installed, query_control, query_service_state,
};
use touchpad_win_service::{
    ControlRequest, SERVICE_DESCRIPTION, SERVICE_DISPLAY_NAME, SERVICE_NAME,
};
use windows_service::service::{
    Service, ServiceAccess, ServiceAction, ServiceActionType, ServiceErrorControl,
    ServiceFailureActions, ServiceFailureResetPeriod, ServiceInfo, ServiceStartType, ServiceState,
    ServiceType,
};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_sys::Win32::Foundation::{
    ERROR_SERVICE_ALREADY_RUNNING, ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SERVICE_EXISTS,
    ERROR_SERVICE_NOT_ACTIVE,
};

const FIREWALL_UDP_RULE_NAME: &str = "TouchPad Remote Service UDP 45454";
const FIREWALL_PORT_RULE_PREFIX: &str = "TouchPad Remote Service UDP";
const FIREWALL_APP_RULE_NAME: &str = "TouchPad Remote Service";

/// Installs (or updates), migrates configuration, configures firewall, and starts `TouchPadRemoteService`.
pub fn install_service(user_config_override: Option<PathBuf>) -> Result<(), String> {
    let user_cfg_path = user_config_override.unwrap_or_else(default_user_desktop_config_path);
    let program_data_path = windows_service_config_path();

    match migrate_user_config_to_program_data(&user_cfg_path, &program_data_path) {
        Ok(true) => println!(
            "Migrated desktop configuration from '{}' to '{}'.",
            user_cfg_path.display(),
            program_data_path.display()
        ),
        Ok(false) => println!(
            "Using ProgramData configuration at '{}'.",
            program_data_path.display()
        ),
        Err(err) => {
            tracing::warn!(%err, "Configuration migration skipped due to error");
        }
    }

    let port = ConfigStore::<DesktopConfig>::new(program_data_path)
        .map(|store| store.get().port)
        .unwrap_or(DEFAULT_PORT);

    let exe_path = std::env::current_exe()
        .map_err(|err| format!("Failed to locate touchpad-win-service executable: {err}"))?;

    configure_firewall_rules(&exe_path, port);

    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .map_err(|err| format!("Failed to open Service Control Manager: {err}"))?;

    let service_info = ServiceInfo {
        name: OsString::from(SERVICE_NAME),
        display_name: OsString::from(SERVICE_DISPLAY_NAME),
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe_path,
        launch_arguments: Vec::new(),
        dependencies: Vec::new(),
        account_name: None, // LocalSystem
        account_password: None,
    };

    let service = match manager.create_service(
        &service_info,
        ServiceAccess::ALL_ACCESS,
    ) {
        Ok(service) => service,
        Err(windows_service::Error::Winapi(err))
            if err.raw_os_error() == Some(ERROR_SERVICE_EXISTS as i32) =>
        {
            let existing = manager
                .open_service(SERVICE_NAME, ServiceAccess::ALL_ACCESS)
                .map_err(|open_err| format!("Failed to open existing service: {open_err}"))?;
            stop_service_and_wait(&existing, Duration::from_secs(10))?;
            existing
                .change_config(&service_info)
                .map_err(|cfg_err| format!("Failed to update service configuration: {cfg_err}"))?;
            existing
        }
        Err(err) => return Err(format!("Failed to create Windows service: {err}")),
    };

    if let Err(err) = service.set_description(SERVICE_DESCRIPTION) {
        tracing::warn!(%err, "Failed to set service description");
    }
    let failure_actions = ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(86_400)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(3),
            },
            ServiceAction {
                action_type: ServiceActionType::Restart,
                delay: Duration::from_secs(5),
            },
            ServiceAction {
                action_type: ServiceActionType::None,
                delay: Duration::ZERO,
            },
        ]),
    };
    if let Err(err) = service.update_failure_actions(failure_actions) {
        tracing::warn!(%err, "Failed to configure service failure recovery actions");
    }

    match service.start(&[] as &[&str]) {
        Ok(()) => {}
        Err(windows_service::Error::Winapi(err))
            if err.raw_os_error() == Some(ERROR_SERVICE_ALREADY_RUNNING as i32) => {}
        Err(err) => return Err(format!("Failed to start {SERVICE_NAME}: {err}")),
    }

    wait_for_service_state(&service, ServiceState::Running, Duration::from_secs(10))?;
    println!("{SERVICE_NAME} installed and running as LocalSystem.");
    Ok(())
}

/// Stops and removes `TouchPadRemoteService` and its firewall rules cleanly.
pub fn uninstall_service() -> Result<(), String> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .map_err(|err| format!("Failed to open Service Control Manager: {err}"))?;

    let service = match manager.open_service(
        SERVICE_NAME,
        ServiceAccess::STOP | ServiceAccess::QUERY_STATUS | ServiceAccess::DELETE,
    ) {
        Ok(service) => service,
        Err(windows_service::Error::Winapi(err))
            if err.raw_os_error() == Some(ERROR_SERVICE_DOES_NOT_EXIST as i32) =>
        {
            remove_firewall_rules();
            println!("{SERVICE_NAME} is not installed.");
            return Ok(());
        }
        Err(err) => return Err(format!("Failed to open {SERVICE_NAME}: {err}")),
    };

    stop_service_and_wait(&service, Duration::from_secs(15))?;
    service
        .delete()
        .map_err(|err| format!("Failed to delete {SERVICE_NAME}: {err}"))?;

    remove_firewall_rules();
    println!("{SERVICE_NAME} stopped and uninstalled.");
    Ok(())
}

/// Prints service/pipe status and returns exit code `0` when healthy and running, `1` otherwise.
pub fn run_status() -> i32 {
    let installed = is_service_installed();
    let state = query_service_state();
    let running = matches!(state, Some(ServiceState::Running));
    let snapshot_res = query_control(&ControlRequest::GetSnapshot).and_then(|r| r.into_snapshot());

    println!("Service Name : {SERVICE_NAME}");
    println!("Installed    : {installed}");
    println!(
        "SCM State    : {}",
        state.map_or_else(|| "NotInstalled".to_string(), |s| format!("{s:?}"))
    );

    match &snapshot_res {
        Ok(snapshot) => {
            println!("Control Pipe : Connected");
            println!("UDP Running  : {}", snapshot.running);
            println!("UDP Port     : {}", snapshot.actual_port);
            println!("Packets      : {}", snapshot.packet_count);
            println!("Paired Count : {}", snapshot.approved_clients.len());
            println!("Pending Count: {}", snapshot.pending_pairings.len());
        }
        Err(err) => {
            println!("Control Pipe : Unavailable ({err})");
        }
    }

    if installed && running && snapshot_res.is_ok() {
        0
    } else {
        1
    }
}

fn stop_service_and_wait(service: &Service, timeout: Duration) -> Result<(), String> {
    match service.stop() {
        Ok(_) => {}
        Err(windows_service::Error::Winapi(err))
            if err.raw_os_error() == Some(ERROR_SERVICE_NOT_ACTIVE as i32) =>
        {
            return Ok(());
        }
        Err(err) => return Err(format!("Failed to stop {SERVICE_NAME}: {err}")),
    }

    wait_for_service_state(service, ServiceState::Stopped, timeout)
}

fn wait_for_service_state(
    service: &Service,
    desired: ServiceState,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let status = service
            .query_status()
            .map_err(|err| format!("Failed to query status for {SERVICE_NAME}: {err}"))?;
        if status.current_state == desired {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Timed out waiting for {SERVICE_NAME} to reach {desired:?} (current: {:?})",
                status.current_state
            ));
        }
        thread::sleep(Duration::from_millis(200));
    }
}

fn configure_firewall_rules(exe_path: &Path, configured_port: u16) {
    remove_firewall_rules();

    let exe_arg = format!("program={}", exe_path.display());
    run_netsh(&[
        "advfirewall",
        "firewall",
        "add",
        "rule",
        &format!("name={FIREWALL_APP_RULE_NAME}"),
        "dir=in",
        "action=allow",
        &exe_arg,
        "enable=yes",
        "profile=any",
    ]);

    let default_port_arg = format!("localport={DEFAULT_PORT}");
    run_netsh(&[
        "advfirewall",
        "firewall",
        "add",
        "rule",
        &format!("name={FIREWALL_UDP_RULE_NAME}"),
        "dir=in",
        "action=allow",
        "protocol=UDP",
        &default_port_arg,
        "enable=yes",
        "profile=any",
    ]);

    if configured_port != DEFAULT_PORT {
        let custom_rule = format!("{FIREWALL_PORT_RULE_PREFIX} {configured_port}");
        let custom_port_arg = format!("localport={configured_port}");
        run_netsh(&[
            "advfirewall",
            "firewall",
            "add",
            "rule",
            &format!("name={custom_rule}"),
            "dir=in",
            "action=allow",
            "protocol=UDP",
            &custom_port_arg,
            "enable=yes",
            "profile=any",
        ]);
    }
}

fn remove_firewall_rules() {
    for name in [FIREWALL_APP_RULE_NAME, FIREWALL_UDP_RULE_NAME] {
        let _ = Command::new("netsh")
            .args([
                "advfirewall",
                "firewall",
                "delete",
                "rule",
                &format!("name={name}"),
            ])
            .output();
    }
}

fn run_netsh(args: &[&str]) {
    match Command::new("netsh").args(args).output() {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            tracing::warn!(?args, %stdout, %stderr, "netsh firewall command returned non-zero");
        }
        Err(err) => {
            tracing::warn!(?args, %err, "Failed to run netsh");
        }
    }
}
