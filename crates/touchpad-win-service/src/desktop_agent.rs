//! Console-session SYSTEM input agent (`--desktop-agent`).
//!
//! Runs input injection on a dedicated windowless thread that switches desktops dynamically
//! via `OpenInputDesktop` + `SetThreadDesktop` across `Default`, `Winlogon` (UAC), and `Screen-saver`.

use std::ptr::null_mut;
use std::thread;
use std::time::{Duration, Instant};

use touchpad_input_win::{is_running_as_system, WindowsInputBackend};
use touchpad_server_core::InputBackend;
use touchpad_win_service::pipe::connect_pipe_client;
use touchpad_win_service::{InputCmd, InputReply, INPUT_PIPE_NAME};
use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_INSUFFICIENT_BUFFER, GENERIC_ALL, HANDLE,
};
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, SetThreadDesktop,
    DESKTOP_READOBJECTS, DESKTOP_SWITCHDESKTOP, DESKTOP_WRITEOBJECTS, HDESK, UOI_NAME,
};

const CONNECT_RETRY_TIMEOUT: Duration = Duration::from_secs(15);
const CONNECT_RETRY_SLEEP: Duration = Duration::from_millis(200);
const INPUT_READ_TIMEOUT_MS: u32 = 60_000;
const INPUT_WRITE_TIMEOUT_MS: u32 = 2_000;
const MAXIMUM_ALLOWED_ACCESS: u32 = 0x0200_0000;

/// Runs the desktop agent loop on a dedicated windowless thread until the pipe closes or disconnects.
pub fn run_desktop_agent() -> Result<(), String> {
    tracing::info!(
        system_token = is_running_as_system(),
        "Starting TouchPad Remote desktop input agent"
    );

    // Dedicated windowless thread: no windows, hooks, or DDE may ever be created on it
    // so SetThreadDesktop never fails with ERROR_BUSY.
    let handle = thread::Builder::new()
        .name("touchpad-input-exec".to_string())
        .spawn(desktop_agent_thread_main)
        .map_err(|err| format!("Failed to spawn desktop agent thread: {err}"))?;

    match handle.join() {
        Ok(res) => res,
        Err(_) => Err("Desktop agent thread panicked".to_string()),
    }
}

fn desktop_agent_thread_main() -> Result<(), String> {
    let mut pipe = connect_with_retry()?;
    pipe.require_system_owner()?;
    tracing::info!("Desktop agent connected to {}", INPUT_PIPE_NAME);

    let backend = WindowsInputBackend::new();
    let mut desktop_tracker = DesktopTracker::default();

    loop {
        let cmd = match pipe.read_json_line::<InputCmd>(INPUT_READ_TIMEOUT_MS, None) {
            Ok(cmd) => cmd,
            Err(err) if err.contains("timed out") => continue,
            Err(err) => {
                tracing::info!(%err, "Desktop agent input pipe closed; exiting");
                return Ok(());
            }
        };

        desktop_tracker.ensure_input_desktop();
        let reply = execute_input_cmd(&backend, cmd);
        if let Err(err) = pipe.write_json_line(&reply, INPUT_WRITE_TIMEOUT_MS) {
            tracing::info!(%err, "Desktop agent write failed; exiting");
            return Ok(());
        }
    }
}

fn connect_with_retry() -> Result<touchpad_win_service::pipe::NamedPipeStream, String> {
    let deadline = Instant::now() + CONNECT_RETRY_TIMEOUT;
    loop {
        match connect_pipe_client(INPUT_PIPE_NAME, 1_000) {
            Ok(stream) => return Ok(stream),
            Err(err) => {
                if Instant::now() >= deadline {
                    return Err(format!(
                        "Desktop agent failed to connect to {INPUT_PIPE_NAME}: {err}"
                    ));
                }
                if err.contains("ACCESS_DENIED") {
                    return Err(err);
                }
                thread::sleep(CONNECT_RETRY_SLEEP);
            }
        }
    }
}

fn execute_input_cmd(backend: &WindowsInputBackend, cmd: InputCmd) -> InputReply {
    match cmd {
        InputCmd::MoveRelative { dx, dy } => {
            backend.move_relative(dx, dy);
            InputReply::Ack
        }
        InputCmd::SetCursorPos { x, y } => {
            backend.set_cursor_pos(x, y);
            InputReply::Ack
        }
        InputCmd::ButtonPress { button } => {
            backend.button_press(button);
            InputReply::Ack
        }
        InputCmd::ButtonRelease { button } => {
            backend.button_release(button);
            InputReply::Ack
        }
        InputCmd::Scroll { ticks } => {
            backend.scroll(ticks);
            InputReply::Ack
        }
        InputCmd::CursorPos => InputReply::CursorPos(backend.cursor_pos()),
        InputCmd::MonitorBounds => InputReply::MonitorBounds(backend.monitor_bounds()),
        InputCmd::IsLockedForGame => InputReply::LockedForGame(backend.is_cursor_locked_for_game()),
    }
}

#[derive(Default)]
struct DesktopTracker {
    current_desktop: Option<HDESK>,
    current_name: Option<String>,
}

impl DesktopTracker {
    fn ensure_input_desktop(&mut self) {
        let Some(new_desk) = open_current_input_desktop() else {
            return;
        };

        let new_name = query_desktop_name(new_desk);
        if let (Some(curr), Some(next)) = (&self.current_name, &new_name) {
            if curr == next {
                unsafe { CloseDesktop(new_desk) };
                return;
            }
        }

        if unsafe { SetThreadDesktop(new_desk) } != 0 {
            if let Some(old_desk) = self.current_desktop.replace(new_desk) {
                unsafe { CloseDesktop(old_desk) };
            }
            tracing::debug!(desktop = ?new_name, "Switched input thread desktop");
            self.current_name = new_name;
        } else {
            let err = unsafe { GetLastError() };
            tracing::warn!(
                win32_err = err,
                desktop = ?new_name,
                "SetThreadDesktop failed"
            );
            unsafe { CloseDesktop(new_desk) };
        }
    }
}

impl Drop for DesktopTracker {
    fn drop(&mut self) {
        if let Some(desk) = self.current_desktop.take() {
            unsafe { CloseDesktop(desk) };
        }
    }
}

fn open_current_input_desktop() -> Option<HDESK> {
    let desired_rights = [
        GENERIC_ALL,
        DESKTOP_READOBJECTS | DESKTOP_WRITEOBJECTS | DESKTOP_SWITCHDESKTOP,
        MAXIMUM_ALLOWED_ACCESS,
    ];
    for rights in desired_rights {
        let desk = unsafe { OpenInputDesktop(0, 0, rights) };
        if !desk.is_null() {
            return Some(desk);
        }
    }
    let err = unsafe { GetLastError() };
    tracing::debug!(win32_err = err, "OpenInputDesktop failed");
    None
}

fn query_desktop_name(desk: HDESK) -> Option<String> {
    let mut needed: u32 = 0;
    let mut buf = vec![0u16; 128];
    let ok = unsafe {
        GetUserObjectInformationW(
            desk as HANDLE,
            UOI_NAME,
            buf.as_mut_ptr().cast(),
            (buf.len() * std::mem::size_of::<u16>()) as u32,
            &mut needed,
        )
    };
    if ok == 0 {
        if unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER || needed == 0 {
            return None;
        }
        let words = (needed as usize).div_ceil(std::mem::size_of::<u16>());
        buf.resize(words, 0);
        let retry = unsafe {
            GetUserObjectInformationW(
                desk as HANDLE,
                UOI_NAME,
                buf.as_mut_ptr().cast(),
                needed,
                null_mut(),
            )
        };
        if retry == 0 {
            return None;
        }
    }

    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}
