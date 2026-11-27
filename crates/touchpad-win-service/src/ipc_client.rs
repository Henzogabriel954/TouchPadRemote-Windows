//! Client helpers for querying and managing `TouchPadRemoteService` from `touchpad-desktop-gui`.

use std::path::Path;

use crate::protocol::{ControlRequest, ControlResponse};
use crate::CONTROL_PIPE_NAME;

#[cfg(windows)]
use crate::SERVICE_NAME;

/// Returns `true` if `TouchPadRemoteService` is registered with the Windows SCM.
pub fn is_service_installed() -> bool {
    #[cfg(windows)]
    {
        query_service_state().is_some()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Returns `true` if `TouchPadRemoteService` is currently in the `Running` state.
pub fn is_service_running() -> bool {
    #[cfg(windows)]
    {
        matches!(
            query_service_state(),
            Some(windows_service::service::ServiceState::Running)
        )
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Queries the current SCM state of `TouchPadRemoteService`, returning `None` if not installed or inaccessible.
#[cfg(windows)]
pub fn query_service_state() -> Option<windows_service::service::ServiceState> {
    use windows_service::service::ServiceAccess;
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};

    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT).ok()?;
    let service = manager
        .open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS)
        .ok()?;
    let status = service.query_status().ok()?;
    Some(status.current_state)
}

/// Sends a [`ControlRequest`] over `\\.\pipe\TouchPadRemoteControl` and returns the [`ControlResponse`].
pub fn query_control(req: &ControlRequest) -> Result<ControlResponse, String> {
    query_control_on_pipe(CONTROL_PIPE_NAME, req)
}

/// Sends a [`ControlRequest`] over the specified named pipe path and returns the [`ControlResponse`].
pub fn query_control_on_pipe(pipe_name: &str, req: &ControlRequest) -> Result<ControlResponse, String> {
    #[cfg(windows)]
    {
        use crate::pipe::connect_pipe_client;

        let mut pipe = connect_pipe_client(pipe_name, 2_000)?;
        pipe.write_json_line(req, 2_000)?;
        pipe.read_json_line::<ControlResponse>(3_000, None)
    }
    #[cfg(not(windows))]
    {
        let _ = (pipe_name, req);
        Err("Windows service control IPC is only supported on Windows".to_string())
    }
}

/// Launches `exe_path --install` with UAC elevation (`ShellExecuteW("runas")`).
pub fn install_service_elevated(exe_path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let user_cfg = touchpad_storage::default_user_desktop_config_path();
        if user_cfg.exists() {
            let user_cfg_str = user_cfg.to_string_lossy().into_owned();
            shell_execute_runas(exe_path, &["--install", "--user-config", &user_cfg_str])
        } else {
            shell_execute_runas(exe_path, &["--install"])
        }
    }
    #[cfg(not(windows))]
    {
        let _ = exe_path;
        Err("Windows service installation is only supported on Windows".to_string())
    }
}

/// Launches `exe_path --uninstall` with UAC elevation (`ShellExecuteW("runas")`).
pub fn uninstall_service_elevated(exe_path: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        shell_execute_runas(exe_path, &["--uninstall"])
    }
    #[cfg(not(windows))]
    {
        let _ = exe_path;
        Err("Windows service uninstallation is only supported on Windows".to_string())
    }
}

#[cfg(windows)]
fn shell_execute_runas(exe_path: &Path, args: &[&str]) -> Result<(), String> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

    use crate::pipe::to_wide_null;

    if exe_path.as_os_str().is_empty() {
        return Err("Executable path must not be empty".to_string());
    }

    let wide_exe = to_wide_null(exe_path.as_os_str())?;
    let wide_verb = to_wide_null("runas")?;
    let joined_args = args
        .iter()
        .map(|arg| quote_windows_arg(arg))
        .collect::<Result<Vec<_>, _>>()?
        .join(" ");
    let wide_params = to_wide_null(joined_args)?;

    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            wide_verb.as_ptr(),
            wide_exe.as_ptr(),
            wide_params.as_ptr(),
            null(),
            SW_HIDE,
        )
    } as isize;

    if result > 32 {
        Ok(())
    } else {
        Err(format!("ShellExecuteW (runas) failed with code {result}"))
    }
}

#[cfg(windows)]
fn quote_windows_arg(arg: &str) -> Result<String, String> {
    if arg.contains('\0') {
        return Err("Argument contains a NUL character".to_string());
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for ch in arg.chars() {
        if ch == '\\' {
            backslashes += 1;
            continue;
        }
        let count = if ch == '"' {
            backslashes * 2 + 1
        } else {
            backslashes
        };
        quoted.extend(std::iter::repeat('\\').take(count));
        quoted.push(ch);
        backslashes = 0;
    }
    quoted.extend(std::iter::repeat('\\').take(backslashes * 2));
    quoted.push('"');
    Ok(quoted)
}
