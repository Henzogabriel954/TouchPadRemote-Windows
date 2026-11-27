use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};

use touchpad_protocol::{HostOs, MouseButton, PrivilegeMode};
use touchpad_server_core::{InputBackend, MonitorRect};
use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HANDLE, LPARAM, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, IsWellKnownSid, TokenElevation, TokenUser, WinLocalSystemSid,
    TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN,
    MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_WHEEL, MOUSEINPUT,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClipCursor, GetCursorInfo, GetCursorPos, SetCursorPos, CURSORINFO, CURSOR_SHOWING,
    SW_SHOWNORMAL,
};

use super::{is_clip_rect_locked_for_game, DEFAULT_MONITOR};

const WHEEL_DELTA: i32 = 120;

#[derive(Debug, Default)]
pub struct WindowsInputBackend;

impl WindowsInputBackend {
    pub fn new() -> Self {
        Self
    }
}

impl InputBackend for WindowsInputBackend {
    fn move_relative(&self, dx: i32, dy: i32) {
        send_mouse_input(MOUSEEVENTF_MOVE, dx, dy, 0);
    }

    fn move_absolute(&self, x: i32, y: i32) {
        self.set_cursor_pos(x, y);
    }

    fn set_cursor_pos(&self, x: i32, y: i32) {
        unsafe { SetCursorPos(x, y) };
    }

    fn button_press(&self, button: MouseButton) {
        let flags = match button {
            MouseButton::Left => MOUSEEVENTF_LEFTDOWN,
            MouseButton::Right => MOUSEEVENTF_RIGHTDOWN,
            MouseButton::Middle => MOUSEEVENTF_MIDDLEDOWN,
        };
        send_mouse_input(flags, 0, 0, 0);
    }

    fn button_release(&self, button: MouseButton) {
        let flags = match button {
            MouseButton::Left => MOUSEEVENTF_LEFTUP,
            MouseButton::Right => MOUSEEVENTF_RIGHTUP,
            MouseButton::Middle => MOUSEEVENTF_MIDDLEUP,
        };
        send_mouse_input(flags, 0, 0, 0);
    }

    fn scroll(&self, ticks: i32) {
        send_mouse_input(MOUSEEVENTF_WHEEL, 0, 0, wheel_data(ticks));
    }

    fn cursor_pos(&self) -> Option<(i32, i32)> {
        let mut point = POINT { x: 0, y: 0 };
        (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x, point.y))
    }

    fn monitor_bounds(&self) -> Vec<MonitorRect> {
        let mut monitors = Vec::new();
        // EnumDisplayMonitors invokes the callback synchronously.
        unsafe {
            EnumDisplayMonitors(
                null_mut(),
                null(),
                Some(collect_monitor),
                (&mut monitors as *mut Vec<MonitorRect>) as LPARAM,
            );
        }
        with_monitor_fallback(monitors)
    }

    fn is_cursor_locked_for_game(&self) -> bool {
        let mut cursor: CURSORINFO = unsafe { zeroed() };
        cursor.cbSize = size_of::<CURSORINFO>() as u32;
        if unsafe { GetCursorInfo(&mut cursor) } != 0 && cursor.flags & CURSOR_SHOWING == 0 {
            return true;
        }

        let mut clip: RECT = unsafe { zeroed() };
        (unsafe { GetClipCursor(&mut clip) } != 0)
            && is_clip_rect_locked_for_game(rect_from_win32(clip), &self.monitor_bounds())
    }

    fn os_kind(&self) -> HostOs {
        HostOs::Windows
    }

    fn privilege_mode(&self) -> PrivilegeMode {
        if is_running_as_system() {
            PrivilegeMode::System
        } else if is_process_elevated() {
            PrivilegeMode::Admin
        } else {
            PrivilegeMode::User
        }
    }
}

fn send_mouse_input(flags: u32, dx: i32, dy: i32, data: u32) {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) };
}

fn wheel_data(ticks: i32) -> u32 {
    // Preserve the legacy Windows direction and JVM Int overflow behavior.
    ticks.wrapping_mul(-WHEEL_DELTA) as u32
}

unsafe extern "system" fn collect_monitor(
    monitor: HMONITOR,
    _: HDC,
    _: *mut RECT,
    data: LPARAM,
) -> BOOL {
    let mut info: MONITORINFO = unsafe { zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info) } != 0 {
        let rect = rect_from_win32(info.rcMonitor);
        if rect.width > 0 && rect.height > 0 {
            // The caller passes an exclusive pointer valid for the whole enumeration.
            unsafe { &mut *(data as *mut Vec<MonitorRect>) }.push(rect);
        }
    }
    1
}

fn rect_from_win32(rect: RECT) -> MonitorRect {
    MonitorRect {
        x: rect.left,
        y: rect.top,
        width: rect.right.saturating_sub(rect.left),
        height: rect.bottom.saturating_sub(rect.top),
    }
}

fn with_monitor_fallback(mut monitors: Vec<MonitorRect>) -> Vec<MonitorRect> {
    if monitors.is_empty() {
        monitors.push(DEFAULT_MONITOR);
    }
    monitors
}

struct ProcessToken(HANDLE);

impl ProcessToken {
    fn open() -> Option<Self> {
        let mut handle = null_mut();
        (unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) } != 0)
            .then(|| Self(handle))
    }
}

impl Drop for ProcessToken {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

pub fn is_process_elevated() -> bool {
    let Some(token) = ProcessToken::open() else {
        return false;
    };
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned = 0;
    let result = unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
    };
    result != 0 && elevation.TokenIsElevated != 0
}

pub fn is_running_as_system() -> bool {
    let Some(token) = ProcessToken::open() else {
        return false;
    };

    let mut bytes = 0;
    unsafe { GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut bytes) };
    if (bytes as usize) < size_of::<TOKEN_USER>() {
        return false;
    }

    // The buffer must align TOKEN_USER and retain the SID it points into.
    let mut buffer = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
    let result = unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            bytes,
            &mut bytes,
        )
    };
    result != 0
        && unsafe {
            let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
            IsWellKnownSid(user.User.Sid, WinLocalSystemSid) != 0
        }
}

/// Launch the current executable via UAC. Supply only arguments after the executable name.
pub fn relaunch_as_admin(args: &[&str]) -> Result<(), String> {
    if args.iter().any(|arg| arg.contains('\0')) {
        return Err("An argument contains a NUL character".into());
    }

    let executable = std::env::current_exe().map_err(|err| err.to_string())?;
    let executable: Vec<u16> = executable
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let verb: Vec<u16> = "runas".encode_utf16().chain(Some(0)).collect();
    let parameters: Vec<u16> = args
        .iter()
        .map(|arg| quote_windows_arg(arg))
        .collect::<Vec<_>>()
        .join(" ")
        .encode_utf16()
        .chain(Some(0))
        .collect();

    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            verb.as_ptr(),
            executable.as_ptr(),
            parameters.as_ptr(),
            null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result > 32 {
        Ok(())
    } else {
        Err(format!("ShellExecuteW (runas) failed with code {result}"))
    }
}

fn quote_windows_arg(arg: &str) -> String {
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
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_conversion_preserves_negative_origin() {
        let monitor = rect_from_win32(RECT {
            left: -1600,
            top: -900,
            right: 0,
            bottom: 0,
        });
        assert_eq!(
            (monitor.x, monitor.y, monitor.width, monitor.height),
            (-1600, -900, 1600, 900)
        );
        assert_eq!(with_monitor_fallback(vec![monitor]), vec![monitor]);
        assert_eq!(with_monitor_fallback(Vec::new()), vec![DEFAULT_MONITOR]);
    }

    #[test]
    fn live_backend_reports_monitors_and_process_privilege() {
        let backend = WindowsInputBackend::new();
        assert_eq!(backend.os_kind(), HostOs::Windows);
        assert!(ProcessToken::open().is_some());
        let expected = match (is_running_as_system(), is_process_elevated()) {
            (true, _) => PrivilegeMode::System,
            (false, true) => PrivilegeMode::Admin,
            (false, false) => PrivilegeMode::User,
        };
        assert_eq!(backend.privilege_mode(), expected);
        let monitors = backend.monitor_bounds();
        assert!(!monitors.is_empty());
        assert!(monitors
            .iter()
            .all(|monitor| monitor.width > 0 && monitor.height > 0));
    }

    #[test]
    fn wheel_data_preserves_windows_direction_and_signed_bits() {
        assert_eq!(wheel_data(2), (-240i32) as u32);
        assert_eq!(wheel_data(-2), 240);
        assert_eq!(wheel_data(0), 0);
        assert_eq!(wheel_data(i32::MIN), 0);
    }

    #[test]
    fn elevation_arguments_escape_quotes_and_trailing_backslashes() {
        assert_eq!(quote_windows_arg(""), r#""""#);
        assert_eq!(quote_windows_arg("two words"), r#""two words""#);
        assert_eq!(quote_windows_arg(r#"a\"b"#), r#""a\\\"b""#);
        assert_eq!(quote_windows_arg(r"C:\with space\"), r#""C:\with space\\""#);
        assert!(relaunch_as_admin(&["bad\0arg"]).is_err());
    }
}
