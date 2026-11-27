//! Session 0 Windows Service runtime driven by the Service Control Manager.

use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use touchpad_server_core::InputBackend;
use touchpad_storage::{windows_service_config_path, ConfigStore, DesktopConfig};
use touchpad_win_service::{
    ControlPipeServer, InputPipeServer, ManagedServer, CONTROL_PIPE_NAME, CONTROL_PIPE_SDDL,
    INPUT_PIPE_NAME, SERVICE_NAME,
};
use touchpad_win_service::pipe::SYSTEM_INPUT_PIPE_SDDL;
use windows_service::define_windows_service;
use windows_service::service::{
    ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
    ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_dispatcher;

use crate::session_monitor::SessionMonitor;

define_windows_service!(ffi_service_main, service_main);

/// Registers the service entrypoint with Windows SCM and blocks until the service stops.
pub fn run_service_dispatcher() -> Result<(), String> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
        .map_err(|err| format!("StartServiceCtrlDispatcherW failed for {SERVICE_NAME}: {err}"))
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(err) = run_service() {
        tracing::error!(%err, "TouchPadRemoteService terminated with error");
    }
}

fn run_service() -> Result<(), String> {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let shutdown_sync = Arc::new((Mutex::new(false), Condvar::new()));
    let session_wake = Arc::new((Mutex::new(0u64), Condvar::new()));

    let handler_stop = Arc::clone(&stop_flag);
    let handler_shutdown = Arc::clone(&shutdown_sync);
    let handler_session = Arc::clone(&session_wake);

    let status_handle = service_control_handler::register(SERVICE_NAME, move |control_event| {
        match control_event {
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            ServiceControl::Stop | ServiceControl::Shutdown => {
                handler_stop.store(true, Ordering::Release);
                {
                    let (lock, cv) = &*handler_shutdown;
                    let mut stopped = lock.lock().unwrap_or_else(|p| p.into_inner());
                    *stopped = true;
                    cv.notify_all();
                }
                {
                    let (lock, cv) = &*handler_session;
                    let mut seq = lock.lock().unwrap_or_else(|p| p.into_inner());
                    *seq = seq.wrapping_add(1);
                    cv.notify_all();
                }
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::SessionChange(_) => {
                let (lock, cv) = &*handler_session;
                let mut seq = lock.lock().unwrap_or_else(|p| p.into_inner());
                *seq = seq.wrapping_add(1);
                cv.notify_all();
                ServiceControlHandlerResult::NoError
            }
            _ => ServiceControlHandlerResult::NotImplemented,
        }
    })
    .map_err(|err| format!("RegisterServiceCtrlHandlerExW failed: {err}"))?;

    let set_status = |state: ServiceState,
                      controls_accepted: ServiceControlAccept,
                      exit_code: ServiceExitCode,
                      checkpoint: u32,
                      wait_hint: Duration| {
        if let Err(err) = status_handle.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted,
            exit_code,
            checkpoint,
            wait_hint,
            process_id: None,
        }) {
            tracing::warn!(%err, ?state, "Failed to update SCM service status");
        }
    };

    set_status(
        ServiceState::StartPending,
        ServiceControlAccept::empty(),
        ServiceExitCode::NO_ERROR,
        1,
        Duration::from_secs(5),
    );

    let startup_result = (|| -> Result<_, String> {
        let config_path = windows_service_config_path();
        let store = Arc::new(
            ConfigStore::<DesktopConfig>::new(config_path)
                .map_err(|err| format!("Failed to load service configuration: {err}"))?,
        );

        let (backend, input_pipe_server) = InputPipeServer::start(
            INPUT_PIPE_NAME.to_string(),
            Some(SYSTEM_INPUT_PIPE_SDDL.to_string()),
            Arc::clone(&stop_flag),
        )?;

        let mut managed_server =
            ManagedServer::new(Arc::clone(&backend) as Arc<dyn InputBackend>, store);
        if let Err(err) = managed_server.start() {
            tracing::error!(%err, "TouchpadServer failed to bind UDP socket on startup");
        }
        let managed = Arc::new(Mutex::new(managed_server));

        let control_pipe_server = ControlPipeServer::start(
            CONTROL_PIPE_NAME.to_string(),
            Some(CONTROL_PIPE_SDDL.to_string()),
            Arc::clone(&managed),
            Arc::clone(&stop_flag),
        )?;

        let session_monitor =
            SessionMonitor::start(Arc::clone(&stop_flag), Arc::clone(&session_wake))?;

        Ok((
            input_pipe_server,
            managed,
            control_pipe_server,
            session_monitor,
        ))
    })();

    let (input_pipe_server, managed, control_pipe_server, session_monitor) = match startup_result {
        Ok(components) => components,
        Err(err) => {
            set_status(
                ServiceState::Stopped,
                ServiceControlAccept::empty(),
                ServiceExitCode::Win32(1),
                0,
                Duration::ZERO,
            );
            return Err(err);
        }
    };

    set_status(
        ServiceState::Running,
        ServiceControlAccept::STOP
            | ServiceControlAccept::SHUTDOWN
            | ServiceControlAccept::SESSION_CHANGE,
        ServiceExitCode::NO_ERROR,
        0,
        Duration::ZERO,
    );
    tracing::info!("{SERVICE_NAME} running in Session 0");

    {
        let (lock, cv) = &*shutdown_sync;
        let mut stopped = lock.lock().unwrap_or_else(|p| p.into_inner());
        while !*stopped && !stop_flag.load(Ordering::Acquire) {
            stopped = cv.wait(stopped).unwrap_or_else(|p| p.into_inner());
        }
    }

    set_status(
        ServiceState::StopPending,
        ServiceControlAccept::empty(),
        ServiceExitCode::NO_ERROR,
        1,
        Duration::from_secs(5),
    );
    tracing::info!("Stopping {SERVICE_NAME}");

    session_monitor.stop_and_join();
    control_pipe_server.stop_and_join();
    managed
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .stop();
    input_pipe_server.stop_and_join();

    set_status(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        ServiceExitCode::NO_ERROR,
        0,
        Duration::ZERO,
    );

    Ok(())
}
