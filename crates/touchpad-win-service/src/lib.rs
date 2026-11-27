//! Windows LocalSystem service IPC library and shared client/server types.

pub mod control_server;
pub mod ipc_client;
pub mod protocol;

#[cfg(windows)]
pub mod pipe;
#[cfg(windows)]
pub mod pipe_backend;

pub const SERVICE_NAME: &str = "TouchPadRemoteService";
pub const SERVICE_DISPLAY_NAME: &str = "TouchPad Remote Input Service";
pub const SERVICE_DESCRIPTION: &str =
    "Receives TouchPad Remote LAN input and forwards cursor events across elevated and UAC desktops.";

pub const CONTROL_PIPE_NAME: &str = r"\\.\pipe\TouchPadRemoteControl";
pub const INPUT_PIPE_NAME: &str = r"\\.\pipe\TouchPadRemoteSysInput";

pub use control_server::ManagedServer;
#[cfg(windows)]
pub use control_server::ControlPipeServer;
#[cfg(windows)]
pub use pipe::{validate_sddl, CONTROL_PIPE_SDDL, SYSTEM_ONLY_SDDL};
#[cfg(windows)]
pub use pipe_backend::{InputPipeServer, PipeInputBackend};
pub use protocol::{
    decode_json_line, encode_json_line, ControlRequest, ControlResponse, InputCmd, InputReply,
    MAX_IPC_LINE_BYTES,
};
