//! Windows Named Pipe transport and SDDL security descriptor helpers.

use std::ffi::OsStr;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde::Serialize;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE,
    ERROR_FILE_NOT_FOUND, ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
    ERROR_PIPE_NOT_CONNECTED, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
    SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    IsWellKnownSid, WinLocalSystemSid, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    SECURITY_ATTRIBUTES,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
    FILE_WRITE_DATA, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SECURITY_ANONYMOUS, SECURITY_SQOS_PRESENT,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, WaitNamedPipeW, PIPE_READMODE_BYTE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};

use crate::protocol::{decode_json_line, encode_json_line, MAX_IPC_LINE_BYTES};

/// SDDL granting Generic All exclusively to LocalSystem (`SY`).
pub const SYSTEM_ONLY_SDDL: &str = "D:(A;;GA;;;SY)";
/// Private input pipe descriptor with explicit LocalSystem ownership.
pub const SYSTEM_INPUT_PIPE_SDDL: &str = "O:SYD:(A;;GA;;;SY)";

/// SDDL granting Generic All to LocalSystem (`SY`) and Built-in Administrators (`BA`),
/// and Generic Read plus FILE_WRITE_DATA to Interactive Users (`IU`).
/// The combined mask excludes FILE_CREATE_PIPE_INSTANCE.
pub const CONTROL_PIPE_SDDL: &str = "D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x12008b;;;IU)";

const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
const PIPE_DEFAULT_TIMEOUT_MS: u32 = 5_000;
const STOP_POLL_SLICE_MS: u32 = 50;

/// RAII wrapper around a Win32 `PSECURITY_DESCRIPTOR` allocated by SDDL conversion.
pub struct SecurityDescriptor {
    sd: PSECURITY_DESCRIPTOR,
}

impl SecurityDescriptor {
    /// Converts a Windows SDDL string into a binary security descriptor.
    pub fn from_sddl(sddl: &str) -> Result<Self, String> {
        let wide = to_wide_null(sddl)?;
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut(),
            )
        };
        if ok == 0 || sd.is_null() {
            let err = unsafe { GetLastError() };
            return Err(format!(
                "ConvertStringSecurityDescriptorToSecurityDescriptorW failed for '{sddl}' (Win32 error {err})"
            ));
        }
        Ok(Self { sd })
    }

    /// Returns a [`SECURITY_ATTRIBUTES`] referencing this descriptor (`bInheritHandle = FALSE`).
    pub fn as_security_attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.sd,
            bInheritHandle: 0,
        }
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        if !self.sd.is_null() {
            unsafe { LocalFree(self.sd) };
            self.sd = null_mut();
        }
    }
}

/// Validates that an SDDL string is accepted by `ConvertStringSecurityDescriptorToSecurityDescriptorW`.
pub fn validate_sddl(sddl: &str) -> Result<(), String> {
    let _ = SecurityDescriptor::from_sddl(sddl)?;
    Ok(())
}

/// Buffered, overlapped Win32 Named Pipe stream supporting bounded timeouts and clean cancellation.
pub struct NamedPipeStream {
    handle: HANDLE,
    event: HANDLE,
    is_server: bool,
    read_buf: Vec<u8>,
}

// SAFETY: The underlying Win32 pipe and manual-reset event handles are owned exclusively by
// `NamedPipeStream`, never shared across threads concurrently (`&mut self` for I/O), and closed on Drop.
unsafe impl Send for NamedPipeStream {}

impl NamedPipeStream {
    fn from_raw(handle: HANDLE, is_server: bool) -> Result<Self, String> {
        let event = unsafe { CreateEventW(null(), 1, 0, null()) };
        if event.is_null() {
            let err = unsafe { GetLastError() };
            unsafe { CloseHandle(handle) };
            return Err(format!("CreateEventW failed (Win32 error {err})"));
        }
        Ok(Self {
            handle,
            event,
            is_server,
            read_buf: Vec::with_capacity(1024),
        })
    }

    /// Writes `data` completely within `timeout_ms`, cancelling overlapped I/O safely on timeout.
    pub fn write_all_timeout(&mut self, data: &[u8], timeout_ms: u32) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
        let mut offset = 0usize;

        while offset < data.len() {
            let remaining = remaining_ms(deadline)?;
            unsafe { ResetEvent(self.event) };
            let mut ov: OVERLAPPED = unsafe { zeroed() };
            ov.hEvent = self.event;

            let chunk = &data[offset..];
            let chunk_len = u32::try_from(chunk.len()).unwrap_or(u32::MAX);
            let mut written: u32 = 0;

            let ok = unsafe {
                WriteFile(
                    self.handle,
                    chunk.as_ptr(),
                    chunk_len,
                    &mut written,
                    &mut ov,
                )
            };

            if ok != 0 {
                if written == 0 {
                    return Err("WriteFile wrote 0 bytes".to_string());
                }
                offset += written as usize;
                continue;
            }

            let err = unsafe { GetLastError() };
            if err != ERROR_IO_PENDING {
                return Err(format_pipe_error("WriteFile", err));
            }

            let wait = unsafe { WaitForSingleObject(self.event, remaining) };
            if wait == WAIT_OBJECT_0 {
                let mut transferred: u32 = 0;
                let res = unsafe { GetOverlappedResult(self.handle, &ov, &mut transferred, 0) };
                if res == 0 {
                    let res_err = unsafe { GetLastError() };
                    return Err(format_pipe_error("WriteFile(GetOverlappedResult)", res_err));
                }
                if transferred == 0 {
                    return Err("WriteFile completed with 0 bytes".to_string());
                }
                offset += transferred as usize;
            } else {
                // Cancel overlapped write and wait for kernel completion before `ov`/`data` drop.
                unsafe {
                    CancelIoEx(self.handle, &ov);
                    let mut dummy: u32 = 0;
                    GetOverlappedResult(self.handle, &ov, &mut dummy, 1);
                }
                return if wait == WAIT_TIMEOUT {
                    Err("Pipe write timed out".to_string())
                } else {
                    let wait_err = unsafe { GetLastError() };
                    Err(format!("WaitForSingleObject(write) failed ({wait_err})"))
                };
            }
        }

        Ok(())
    }

    /// Reads one `\n`-terminated line up to `MAX_IPC_LINE_BYTES`, honoring `timeout_ms` and optional `stop`.
    pub fn read_line_timeout(
        &mut self,
        timeout_ms: u32,
        stop: Option<&AtomicBool>,
    ) -> Result<Vec<u8>, String> {
        if let Some(line) = take_buffered_line(&mut self.read_buf)? {
            return Ok(line);
        }

        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));
        let mut chunk = [0u8; 4096];

        loop {
            if stop.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                return Err("Pipe read stopped".to_string());
            }

            let _ = remaining_ms(deadline)?;
            unsafe { ResetEvent(self.event) };
            let mut ov: OVERLAPPED = unsafe { zeroed() };
            ov.hEvent = self.event;
            let mut bytes_read: u32 = 0;

            let ok = unsafe {
                ReadFile(
                    self.handle,
                    chunk.as_mut_ptr(),
                    chunk.len() as u32,
                    &mut bytes_read,
                    &mut ov,
                )
            };

            let read_len = if ok != 0 {
                bytes_read as usize
            } else {
                let err = unsafe { GetLastError() };
                if err != ERROR_IO_PENDING {
                    return Err(format_pipe_error("ReadFile", err));
                }
                self.wait_overlapped_read(&ov, deadline, stop)?
            };

            if read_len == 0 {
                return Err("Pipe closed (EOF)".to_string());
            }

            self.read_buf.extend_from_slice(&chunk[..read_len]);
            if let Some(line) = take_buffered_line(&mut self.read_buf)? {
                return Ok(line);
            }
        }
    }

    fn wait_overlapped_read(
        &self,
        ov: &OVERLAPPED,
        deadline: Instant,
        stop: Option<&AtomicBool>,
    ) -> Result<usize, String> {
        loop {
            if stop.is_some_and(|flag| flag.load(Ordering::Acquire)) {
                cancel_overlapped(self.handle, ov);
                return Err("Pipe read stopped".to_string());
            }

            let Ok(rem) = remaining_ms(deadline) else {
                cancel_overlapped(self.handle, ov);
                return Err("Pipe read timed out".to_string());
            };

            let slice = if stop.is_some() {
                rem.min(STOP_POLL_SLICE_MS)
            } else {
                rem
            };

            let wait = unsafe { WaitForSingleObject(self.event, slice) };
            if wait == WAIT_OBJECT_0 {
                let mut transferred: u32 = 0;
                let res = unsafe { GetOverlappedResult(self.handle, ov, &mut transferred, 0) };
                if res == 0 {
                    let err = unsafe { GetLastError() };
                    return Err(format_pipe_error("ReadFile(GetOverlappedResult)", err));
                }
                return Ok(transferred as usize);
            }
            if wait != WAIT_TIMEOUT {
                let err = unsafe { GetLastError() };
                cancel_overlapped(self.handle, ov);
                return Err(format!("WaitForSingleObject(read) failed ({err})"));
            }
        }
    }

    /// Confirms the owner of this connected pipe is LocalSystem.
    pub fn require_system_owner(&self) -> Result<(), String> {
        let mut owner = null_mut();
        let mut descriptor = null_mut();
        let status = unsafe {
            GetSecurityInfo(
                self.handle,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                null_mut(),
                null_mut(),
                &mut descriptor,
            )
        };
        if status != 0 {
            return Err(format!("GetSecurityInfo(pipe owner) failed (Win32 error {status})"));
        }
        let _descriptor = SecurityDescriptor { sd: descriptor };
        let system_owned =
            !owner.is_null() && unsafe { IsWellKnownSid(owner, WinLocalSystemSid) != 0 };
        if !system_owned {
            return Err("Pipe server is not owned by LocalSystem".to_string());
        }
        Ok(())
    }

    /// Serializes `value` as a compact JSON line and writes it within `timeout_ms`.
    pub fn write_json_line<T: Serialize>(
        &mut self,
        value: &T,
        timeout_ms: u32,
    ) -> Result<(), String> {
        let encoded = encode_json_line(value)?;
        self.write_all_timeout(&encoded, timeout_ms)
    }

    /// Reads and deserializes one compact JSON line within `timeout_ms`.
    pub fn read_json_line<T: DeserializeOwned>(
        &mut self,
        timeout_ms: u32,
        stop: Option<&AtomicBool>,
    ) -> Result<T, String> {
        let line = self.read_line_timeout(timeout_ms, stop)?;
        decode_json_line(&line)
    }
}

impl Drop for NamedPipeStream {
    fn drop(&mut self) {
        if self.is_server && !self.handle.is_null() && self.handle != INVALID_HANDLE_VALUE {
            unsafe { DisconnectNamedPipe(self.handle) };
        }
        if !self.handle.is_null() && self.handle != INVALID_HANDLE_VALUE {
            unsafe { CloseHandle(self.handle) };
            self.handle = INVALID_HANDLE_VALUE;
        }
        if !self.event.is_null() {
            unsafe { CloseHandle(self.event) };
            self.event = null_mut();
        }
    }
}

/// Creates a Win32 Named Pipe server instance with remote-client rejection and optional SDDL DACL.
pub fn create_named_pipe_server(
    pipe_name: &str,
    sddl: Option<&str>,
    max_instances: u32,
    first_instance: bool,
) -> Result<NamedPipeStream, String> {
    let wide_name = to_wide_null(pipe_name)?;
    let sd = sddl.map(SecurityDescriptor::from_sddl).transpose()?;
    let sa = sd.as_ref().map(SecurityDescriptor::as_security_attributes);
    let sa_ptr = sa.as_ref().map_or(null(), |attrs| attrs as *const _);

    let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
    if first_instance {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let pipe_mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;

    let handle = unsafe {
        CreateNamedPipeW(
            wide_name.as_ptr(),
            open_mode,
            pipe_mode,
            max_instances,
            PIPE_BUFFER_SIZE,
            PIPE_BUFFER_SIZE,
            PIPE_DEFAULT_TIMEOUT_MS,
            sa_ptr,
        )
    };

    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        let err = unsafe { GetLastError() };
        return Err(format_pipe_error(&format!("CreateNamedPipeW({pipe_name})"), err));
    }

    let stream = NamedPipeStream::from_raw(handle, true)?;
    if sddl == Some(SYSTEM_INPUT_PIPE_SDDL) {
        stream.require_system_owner()?;
    }
    Ok(stream)
}

/// Waits for a client connection on `pipe`, waking every 50 ms to observe `stop`.
///
/// Returns `Ok(true)` when connected or `Ok(false)` when stopped cleanly.
pub fn connect_named_pipe_with_stop(
    pipe: &NamedPipeStream,
    stop: &AtomicBool,
) -> Result<bool, String> {
    if stop.load(Ordering::Acquire) {
        return Ok(false);
    }

    unsafe { ResetEvent(pipe.event) };
    let mut ov: OVERLAPPED = unsafe { zeroed() };
    ov.hEvent = pipe.event;

    let ok = unsafe { ConnectNamedPipe(pipe.handle, &mut ov) };
    if ok != 0 {
        return Ok(true);
    }

    let err = unsafe { GetLastError() };
    if err == ERROR_PIPE_CONNECTED {
        return Ok(true);
    }
    if err != ERROR_IO_PENDING {
        return Err(format_pipe_error("ConnectNamedPipe", err));
    }

    loop {
        if stop.load(Ordering::Acquire) {
            cancel_overlapped(pipe.handle, &ov);
            return Ok(false);
        }

        let wait = unsafe { WaitForSingleObject(pipe.event, STOP_POLL_SLICE_MS) };
        if wait == WAIT_OBJECT_0 {
            let mut dummy: u32 = 0;
            let res = unsafe { GetOverlappedResult(pipe.handle, &ov, &mut dummy, 0) };
            if res != 0 {
                return Ok(true);
            }
            let res_err = unsafe { GetLastError() };
            if res_err == ERROR_PIPE_CONNECTED {
                return Ok(true);
            }
            return Err(format_pipe_error("ConnectNamedPipe(GetOverlappedResult)", res_err));
        }
        if wait != WAIT_TIMEOUT {
            let wait_err = unsafe { GetLastError() };
            cancel_overlapped(pipe.handle, &ov);
            return Err(format!("WaitForSingleObject(connect) failed ({wait_err})"));
        }
    }
}

/// Connects to an existing Win32 Named Pipe as a client within `timeout_ms`.
pub fn connect_pipe_client(pipe_name: &str, timeout_ms: u32) -> Result<NamedPipeStream, String> {
    let wide_name = to_wide_null(pipe_name)?;
    let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_ms));

    loop {
        let handle = unsafe {
            CreateFileW(
                wide_name.as_ptr(),
                GENERIC_READ | FILE_WRITE_DATA,
                0,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_ANONYMOUS,
                null_mut(),
            )
        };

        if !handle.is_null() && handle != INVALID_HANDLE_VALUE {
            return NamedPipeStream::from_raw(handle, false);
        }

        let err = unsafe { GetLastError() };
        if err == ERROR_ACCESS_DENIED || err == ERROR_FILE_NOT_FOUND {
            return Err(format_pipe_error(&format!("CreateFileW({pipe_name})"), err));
        }

        if err != ERROR_PIPE_BUSY {
            return Err(format_pipe_error(&format!("CreateFileW({pipe_name})"), err));
        }

        let Ok(rem) = remaining_ms(deadline) else {
            return Err(format!(
                "Timed out waiting for busy pipe {pipe_name} (ERROR_PIPE_BUSY)"
            ));
        };

        let wait_ms = rem.min(200);
        if unsafe { WaitNamedPipeW(wide_name.as_ptr(), wait_ms) } == 0 {
            let wait_err = unsafe { GetLastError() };
            if wait_err == ERROR_ACCESS_DENIED || wait_err == ERROR_FILE_NOT_FOUND {
                return Err(format_pipe_error(&format!("WaitNamedPipeW({pipe_name})"), wait_err));
            }
            std::thread::sleep(Duration::from_millis(15));
        }
    }
}

fn take_buffered_line(buf: &mut Vec<u8>) -> Result<Option<Vec<u8>>, String> {
    if let Some(pos) = buf.iter().position(|&b| b == b'\n') {
        if pos + 1 > MAX_IPC_LINE_BYTES {
            buf.clear();
            return Err(format!(
                "IPC line exceeded maximum length ({} > {} bytes)",
                pos + 1,
                MAX_IPC_LINE_BYTES
            ));
        }
        let line: Vec<u8> = buf.drain(..=pos).collect();
        return Ok(Some(line));
    }

    if buf.len() > MAX_IPC_LINE_BYTES {
        let len = buf.len();
        buf.clear();
        return Err(format!(
            "Unterminated IPC line exceeded maximum length ({len} > {MAX_IPC_LINE_BYTES} bytes)"
        ));
    }

    Ok(None)
}

fn cancel_overlapped(handle: HANDLE, ov: &OVERLAPPED) {
    unsafe {
        CancelIoEx(handle, ov);
        let mut dummy: u32 = 0;
        GetOverlappedResult(handle, ov, &mut dummy, 1);
    }
}

fn remaining_ms(deadline: Instant) -> Result<u32, String> {
    let now = Instant::now();
    if now >= deadline {
        return Err("Pipe operation timed out".to_string());
    }
    let ms = deadline.duration_since(now).as_millis();
    Ok(u32::try_from(ms).unwrap_or(u32::MAX).max(1))
}

fn format_pipe_error(op: &str, err: u32) -> String {
    match err {
        ERROR_ACCESS_DENIED => format!("{op} failed: ACCESS_DENIED (Win32 error {err})"),
        ERROR_FILE_NOT_FOUND => format!("{op} failed: FILE_NOT_FOUND (Win32 error {err})"),
        ERROR_PIPE_BUSY => format!("{op} failed: PIPE_BUSY (Win32 error {err})"),
        ERROR_BROKEN_PIPE => format!("{op} failed: BROKEN_PIPE (Win32 error {err})"),
        ERROR_NO_DATA => format!("{op} failed: NO_DATA (Win32 error {err})"),
        ERROR_PIPE_NOT_CONNECTED => {
            format!("{op} failed: PIPE_NOT_CONNECTED (Win32 error {err})")
        }
        _ => format!("{op} failed (Win32 error {err})"),
    }
}

/// Converts an OS string into a NUL-terminated UTF-16 buffer, rejecting embedded NULs.
pub fn to_wide_null(value: impl AsRef<OsStr>) -> Result<Vec<u16>, String> {
    let os = value.as_ref();
    let mut wide: Vec<u16> = Vec::with_capacity(os.len() + 1);
    for unit in os.encode_wide() {
        if unit == 0 {
            return Err("String contains embedded NUL character".to_string());
        }
        wide.push(unit);
    }
    wide.push(0);
    Ok(wide)
}
