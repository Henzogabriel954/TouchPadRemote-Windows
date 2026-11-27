//! Session 0 monitor that launches `<self> --desktop-agent` into the active console session.

use std::mem::size_of;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use touchpad_win_service::pipe::to_wide_null;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::Security::{
    DuplicateTokenEx, SecurityIdentification, SetTokenInformation, TokenPrimary, TokenSessionId,
    TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_SESSIONID, TOKEN_ALL_ACCESS, TOKEN_ASSIGN_PRIMARY,
    TOKEN_DUPLICATE, TOKEN_QUERY,
};
use windows_sys::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId;
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, GetCurrentProcess, OpenProcessToken, TerminateProcess,
    WaitForSingleObject, CREATE_NO_WINDOW, NORMAL_PRIORITY_CLASS, PROCESS_INFORMATION,
    STARTF_USESHOWWINDOW, STARTUPINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

const POLL_INTERVAL: Duration = Duration::from_millis(500);
const CRASH_BACKOFF: Duration = Duration::from_millis(500);
const NO_CONSOLE_SESSION: u32 = u32::MAX;

struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
            self.0 = null_mut();
        }
    }
}

struct AgentProcess {
    session_id: u32,
    process: OwnedHandle,
    pid: u32,
}

impl AgentProcess {
    fn has_exited(&self) -> bool {
        unsafe { WaitForSingleObject(self.process.0, 0) == WAIT_OBJECT_0 }
    }

    fn terminate(self) {
        tracing::info!(pid = self.pid, session_id = self.session_id, "Terminating desktop agent");
        unsafe {
            TerminateProcess(self.process.0, 0);
            WaitForSingleObject(self.process.0, 2_000);
        }
    }
}

/// Supervises the active console session's `--desktop-agent` child process.
pub struct SessionMonitor {
    stop_flag: Arc<AtomicBool>,
    wake: Arc<(Mutex<u64>, Condvar)>,
    worker: Option<JoinHandle<()>>,
}

impl SessionMonitor {
    /// Starts the background monitor thread that tracks `WTSGetActiveConsoleSessionId()`.
    pub fn start(
        stop_flag: Arc<AtomicBool>,
        wake: Arc<(Mutex<u64>, Condvar)>,
    ) -> Result<Self, String> {
        let thread_stop = Arc::clone(&stop_flag);
        let thread_wake = Arc::clone(&wake);
        let worker = thread::Builder::new()
            .name("touchpad-session-monitor".to_string())
            .spawn(move || monitor_loop(thread_stop, thread_wake))
            .map_err(|err| format!("Failed to spawn session monitor thread: {err}"))?;

        Ok(Self {
            stop_flag,
            wake,
            worker: Some(worker),
        })
    }

    /// Stops the monitor thread and terminates any running `--desktop-agent`.
    pub fn stop_and_join(mut self) {
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        self.stop_flag.store(true, Ordering::Release);
        {
            let (lock, cv) = &*self.wake;
            let mut seq = lock.lock().unwrap_or_else(|p| p.into_inner());
            *seq = seq.wrapping_add(1);
            cv.notify_all();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for SessionMonitor {
    fn drop(&mut self) {
        self.stop_inner();
    }
}

fn monitor_loop(stop_flag: Arc<AtomicBool>, wake: Arc<(Mutex<u64>, Condvar)>) {
    let mut current: Option<AgentProcess> = None;
    let mut last_seq = *wake.0.lock().unwrap_or_else(|p| p.into_inner());

    while !stop_flag.load(Ordering::Acquire) {
        let active_session = unsafe { WTSGetActiveConsoleSessionId() };

        if active_session == 0 || active_session == NO_CONSOLE_SESSION {
            if let Some(proc) = current.take() {
                proc.terminate();
            }
        } else {
            let need_respawn = match &current {
                Some(proc) if proc.session_id != active_session => {
                    tracing::info!(
                        old_session = proc.session_id,
                        new_session = active_session,
                        "Active console session changed; cycling desktop agent"
                    );
                    if let Some(old) = current.take() {
                        old.terminate();
                    }
                    true
                }
                Some(proc) if proc.has_exited() => {
                    tracing::warn!(
                        pid = proc.pid,
                        session_id = proc.session_id,
                        "Desktop agent exited unexpectedly; restarting after backoff"
                    );
                    current = None;
                    wait_interruptible(&stop_flag, &wake, &mut last_seq, CRASH_BACKOFF);
                    !stop_flag.load(Ordering::Acquire)
                }
                Some(_) => false,
                None => true,
            };

            if need_respawn && !stop_flag.load(Ordering::Acquire) {
                match spawn_desktop_agent(active_session) {
                    Ok(proc) => {
                        tracing::info!(
                            pid = proc.pid,
                            session_id = proc.session_id,
                            "Spawned SYSTEM desktop agent into active console session"
                        );
                        current = Some(proc);
                    }
                    Err(err) => {
                        tracing::error!(session_id = active_session, %err, "Failed to spawn desktop agent");
                    }
                }
            }
        }

        if stop_flag.load(Ordering::Acquire) {
            break;
        }
        wait_interruptible(&stop_flag, &wake, &mut last_seq, POLL_INTERVAL);
    }

    if let Some(proc) = current.take() {
        proc.terminate();
    }
}

fn wait_interruptible(
    stop_flag: &AtomicBool,
    wake: &(Mutex<u64>, Condvar),
    last_seq: &mut u64,
    timeout: Duration,
) {
    let (lock, cv) = wake;
    let guard = lock.lock().unwrap_or_else(|p| p.into_inner());
    if stop_flag.load(Ordering::Acquire) || *guard != *last_seq {
        *last_seq = *guard;
        return;
    }
    let (guard, _) = cv
        .wait_timeout(guard, timeout)
        .unwrap_or_else(|p| p.into_inner());
    *last_seq = *guard;
}

fn spawn_desktop_agent(session_id: u32) -> Result<AgentProcess, String> {
    let exe_path = std::env::current_exe()
        .map_err(|err| format!("current_exe failed: {err}"))?;
    let exe_wide = to_wide_null(exe_path.as_os_str())?;
    let cmd_str = format!("\"{}\" --desktop-agent", exe_path.display());
    let mut cmd_wide = to_wide_null(cmd_str)?;
    let mut desktop_wide = to_wide_null(r"winsta0\default")?;

    let mut raw_token: HANDLE = null_mut();
    let desired_access = TOKEN_DUPLICATE
        | TOKEN_QUERY
        | TOKEN_ASSIGN_PRIMARY
        | TOKEN_ADJUST_DEFAULT
        | TOKEN_ADJUST_SESSIONID;
    if unsafe { OpenProcessToken(GetCurrentProcess(), desired_access, &mut raw_token) } == 0 {
        let err = unsafe { GetLastError() };
        return Err(format!("OpenProcessToken failed (Win32 error {err})"));
    }
    let token = OwnedHandle(raw_token);

    let mut raw_dup: HANDLE = null_mut();
    if unsafe {
        DuplicateTokenEx(
            token.0,
            TOKEN_ALL_ACCESS,
            null(),
            SecurityIdentification,
            TokenPrimary,
            &mut raw_dup,
        )
    } == 0
    {
        let err = unsafe { GetLastError() };
        return Err(format!("DuplicateTokenEx failed (Win32 error {err})"));
    }
    let dup_token = OwnedHandle(raw_dup);

    if unsafe {
        SetTokenInformation(
            dup_token.0,
            TokenSessionId,
            (&session_id as *const u32).cast(),
            size_of::<u32>() as u32,
        )
    } == 0
    {
        let err = unsafe { GetLastError() };
        return Err(format!(
            "SetTokenInformation(TokenSessionId={session_id}) failed (Win32 error {err})"
        ));
    }

    let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
    si.cb = size_of::<STARTUPINFOW>() as u32;
    si.lpDesktop = desktop_wide.as_mut_ptr();
    si.dwFlags = STARTF_USESHOWWINDOW;
    si.wShowWindow = SW_HIDE as u16;

    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    let ok = unsafe {
        CreateProcessAsUserW(
            dup_token.0,
            exe_wide.as_ptr(),
            cmd_wide.as_mut_ptr(),
            null(),
            null(),
            0,
            CREATE_NO_WINDOW | NORMAL_PRIORITY_CLASS,
            null(),
            null(),
            &si,
            &mut pi,
        )
    };

    if ok == 0 {
        let err = unsafe { GetLastError() };
        return Err(format!("CreateProcessAsUserW failed (Win32 error {err})"));
    }

    if !pi.hThread.is_null() {
        unsafe { CloseHandle(pi.hThread) };
    }

    Ok(AgentProcess {
        session_id,
        process: OwnedHandle(pi.hProcess),
        pid: pi.dwProcessId,
    })
}
