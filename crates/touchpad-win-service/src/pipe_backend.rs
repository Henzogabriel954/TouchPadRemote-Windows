//! `InputBackend` proxy over `\\.\pipe\TouchPadRemoteSysInput` and its single-agent server.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use touchpad_protocol::{HostOs, MouseButton, PrivilegeMode};
use touchpad_server_core::{InputBackend, MonitorRect};

use crate::pipe::{connect_named_pipe_with_stop, create_named_pipe_server, NamedPipeStream};
use crate::protocol::{InputCmd, InputReply};

const DEFAULT_IO_TIMEOUT_MS: u32 = 500;
const FALLBACK_MONITOR: MonitorRect = MonitorRect {
    x: 0,
    y: 0,
    width: 1920,
    height: 1080,
};

/// `InputBackend` implementation used by the Session 0 Windows Service to forward
/// cursor, click, scroll, and monitor queries to the active console `--desktop-agent`.
pub struct PipeInputBackend {
    active_pipe: Mutex<Option<NamedPipeStream>>,
    io_timeout_ms: u32,
}

impl PipeInputBackend {
    /// Creates a new disconnected [`PipeInputBackend`] with the given per-command I/O timeout.
    pub fn new(io_timeout_ms: u32) -> Self {
        Self {
            active_pipe: Mutex::new(None),
            io_timeout_ms: io_timeout_ms.max(1),
        }
    }

    /// Replaces the currently active desktop-agent pipe stream.
    pub fn attach_pipe(&self, stream: NamedPipeStream) {
        let mut guard = self
            .active_pipe
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = Some(stream);
    }

    /// Drops the currently attached desktop-agent pipe stream, if any.
    pub fn detach_pipe(&self) {
        let mut guard = self
            .active_pipe
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = None;
    }

    /// Returns `true` when an active desktop-agent pipe connection is attached.
    pub fn is_connected(&self) -> bool {
        self.active_pipe
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_some()
    }

    fn send_cmd(&self, cmd: &InputCmd) -> Option<InputReply> {
        let mut guard = self
            .active_pipe
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let stream = guard.as_mut()?;

        if let Err(err) = stream.write_json_line(cmd, self.io_timeout_ms) {
            tracing::warn!(%err, "Input pipe write failed; detaching desktop agent pipe");
            *guard = None;
            return None;
        }

        match stream.read_json_line::<InputReply>(self.io_timeout_ms, None) {
            Ok(reply) => Some(reply),
            Err(err) => {
                tracing::warn!(%err, "Input pipe read failed; detaching desktop agent pipe");
                *guard = None;
                None
            }
        }
    }
}

impl Default for PipeInputBackend {
    fn default() -> Self {
        Self::new(DEFAULT_IO_TIMEOUT_MS)
    }
}

impl InputBackend for PipeInputBackend {
    fn supports_relative_move(&self) -> bool {
        true
    }

    fn move_relative(&self, dx: i32, dy: i32) {
        let _ = self.send_cmd(&InputCmd::MoveRelative { dx, dy });
    }

    fn move_absolute(&self, x: i32, y: i32) {
        self.set_cursor_pos(x, y);
    }

    fn set_cursor_pos(&self, x: i32, y: i32) {
        let _ = self.send_cmd(&InputCmd::SetCursorPos { x, y });
    }

    fn button_press(&self, button: MouseButton) {
        let _ = self.send_cmd(&InputCmd::ButtonPress { button });
    }

    fn button_release(&self, button: MouseButton) {
        let _ = self.send_cmd(&InputCmd::ButtonRelease { button });
    }

    fn scroll(&self, ticks: i32) {
        let _ = self.send_cmd(&InputCmd::Scroll { ticks });
    }

    fn cursor_pos(&self) -> Option<(i32, i32)> {
        match self.send_cmd(&InputCmd::CursorPos) {
            Some(InputReply::CursorPos(pos)) => pos,
            _ => None,
        }
    }

    fn monitor_bounds(&self) -> Vec<MonitorRect> {
        match self.send_cmd(&InputCmd::MonitorBounds) {
            Some(InputReply::MonitorBounds(bounds)) if !bounds.is_empty() => bounds,
            _ => vec![FALLBACK_MONITOR],
        }
    }

    fn is_cursor_locked_for_game(&self) -> bool {
        matches!(
            self.send_cmd(&InputCmd::IsLockedForGame),
            Some(InputReply::LockedForGame(true))
        )
    }

    fn os_kind(&self) -> HostOs {
        HostOs::Windows
    }

    fn privilege_mode(&self) -> PrivilegeMode {
        PrivilegeMode::System
    }
}

/// Acceptor managing `\\.\pipe\TouchPadRemoteSysInput` for incoming `--desktop-agent` connections.
pub struct InputPipeServer {
    stop_flag: Arc<AtomicBool>,
    backend: Arc<PipeInputBackend>,
    worker: Option<JoinHandle<()>>,
}

impl InputPipeServer {
    /// Synchronously creates the first pipe instance (with `FILE_FLAG_FIRST_PIPE_INSTANCE`)
    /// and starts the acceptor thread feeding [`PipeInputBackend`].
    pub fn start(
        pipe_name: String,
        sddl: Option<String>,
        stop_flag: Arc<AtomicBool>,
    ) -> Result<(Arc<PipeInputBackend>, Self), String> {
        Self::start_with_timeout(pipe_name, sddl, stop_flag, DEFAULT_IO_TIMEOUT_MS)
    }

    /// Starts [`InputPipeServer`] with a custom per-command [`PipeInputBackend`] I/O timeout.
    pub fn start_with_timeout(
        pipe_name: String,
        sddl: Option<String>,
        stop_flag: Arc<AtomicBool>,
        io_timeout_ms: u32,
    ) -> Result<(Arc<PipeInputBackend>, Self), String> {
        const MAX_INSTANCES: u32 = 4;

        let initial_pipe =
            create_named_pipe_server(&pipe_name, sddl.as_deref(), MAX_INSTANCES, true)?;
        let backend = Arc::new(PipeInputBackend::new(io_timeout_ms));

        let thread_backend = Arc::clone(&backend);
        let thread_stop = Arc::clone(&stop_flag);
        let worker = thread::Builder::new()
            .name("touchpad-input-pipe".to_string())
            .spawn(move || {
                let mut current_pipe = Some(initial_pipe);
                while !thread_stop.load(Ordering::Acquire) {
                    let listener = match current_pipe.take() {
                        Some(pipe) => pipe,
                        None => match create_named_pipe_server(
                            &pipe_name,
                            sddl.as_deref(),
                            MAX_INSTANCES,
                            false,
                        ) {
                            Ok(pipe) => pipe,
                            Err(err) => {
                                tracing::warn!(%err, "Retrying input pipe server creation");
                                thread::sleep(Duration::from_millis(50));
                                continue;
                            }
                        },
                    };

                    match connect_named_pipe_with_stop(&listener, &thread_stop) {
                        Ok(true) => {
                            tracing::info!(%pipe_name, "Desktop agent connected to private input pipe");
                            thread_backend.attach_pipe(listener);
                        }
                        Ok(false) => {
                            drop(listener);
                            break;
                        }
                        Err(err) => {
                            tracing::warn!(%err, "ConnectNamedPipe failed on input pipe");
                            drop(listener);
                            thread::sleep(Duration::from_millis(50));
                        }
                    }
                }
                thread_backend.detach_pipe();
            })
            .map_err(|err| format!("Failed to spawn input pipe thread: {err}"))?;

        let server = Self {
            stop_flag,
            backend: Arc::clone(&backend),
            worker: Some(worker),
        };
        Ok((backend, server))
    }

    /// Signals shutdown, detaches the active desktop-agent pipe, and joins the acceptor thread.
    pub fn stop_and_join(mut self) {
        self.stop_inner();
    }

    fn stop_inner(&mut self) {
        self.stop_flag.store(true, Ordering::Release);
        self.backend.detach_pipe();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for InputPipeServer {
    fn drop(&mut self) {
        self.stop_inner();
    }
}
