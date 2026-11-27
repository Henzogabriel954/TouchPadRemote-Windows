//! IPC protocol definitions and line-delimited JSON framing for `touchpad-win-service`.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use touchpad_protocol::MouseButton;
use touchpad_server_core::{MonitorRect, ServerSnapshot};
use touchpad_storage::DesktopConfig;

/// Maximum permitted length (in bytes) of a single line-delimited JSON IPC message.
pub const MAX_IPC_LINE_BYTES: usize = 256 * 1024;

/// Control requests sent by `touchpad-desktop-gui` over `\\.\pipe\TouchPadRemoteControl`.
///
/// SECURITY: This enum intentionally excludes all input injection commands
/// (no cursor movement, click, button state, scroll, or cursor warp).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ControlRequest {
    GetSnapshot,
    ApprovePairing {
        ip_or_id: String,
    },
    RejectPairing {
        ip_or_id: String,
    },
    RevokeDevice {
        client_id: String,
        ip_address: String,
    },
    UpdateConfig(DesktopConfig),
    StartServer,
    StopServer,
}

/// Responses returned by the Windows service over `\\.\pipe\TouchPadRemoteControl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ControlResponse {
    Ok,
    Snapshot(ServerSnapshot),
    Error(String),
}

impl ControlResponse {
    /// Extracts the [`ServerSnapshot`] from a [`ControlResponse::Snapshot`] variant.
    pub fn into_snapshot(self) -> Result<ServerSnapshot, String> {
        match self {
            Self::Snapshot(snapshot) => Ok(snapshot),
            Self::Error(err) => Err(err),
            Self::Ok => Err("Expected Snapshot response, got Ok".to_string()),
        }
    }

    /// Converts [`ControlResponse::Ok`] into `Ok(())` and [`ControlResponse::Error`] into `Err(...)`.
    pub fn into_result(self) -> Result<(), String> {
        match self {
            Self::Ok | Self::Snapshot(_) => Ok(()),
            Self::Error(err) => Err(err),
        }
    }
}

/// Private input commands forwarded from the Session 0 service to the console-session
/// `--desktop-agent` over `\\.\pipe\TouchPadRemoteSysInput` (`LocalSystem` only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputCmd {
    MoveRelative { dx: i32, dy: i32 },
    SetCursorPos { x: i32, y: i32 },
    ButtonPress { button: MouseButton },
    ButtonRelease { button: MouseButton },
    Scroll { ticks: i32 },
    CursorPos,
    MonitorBounds,
    IsLockedForGame,
}

/// Private replies returned by `--desktop-agent` to the Session 0 service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputReply {
    Ack,
    CursorPos(Option<(i32, i32)>),
    MonitorBounds(Vec<MonitorRect>),
    LockedForGame(bool),
    Error(String),
}

/// Encodes a serializable IPC value as a single compact JSON line terminated by `\n`.
pub fn encode_json_line<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec(value).map_err(|err| format!("JSON encode error: {err}"))?;
    if bytes.len() >= MAX_IPC_LINE_BYTES {
        return Err(format!(
            "Encoded IPC line exceeds maximum size ({} >= {} bytes)",
            bytes.len(),
            MAX_IPC_LINE_BYTES
        ));
    }
    bytes.push(b'\n');
    Ok(bytes)
}

/// Decodes a single JSON line (with optional trailing `\r\n`) into `T`.
pub fn decode_json_line<T: DeserializeOwned>(line: &[u8]) -> Result<T, String> {
    if line.len() > MAX_IPC_LINE_BYTES {
        return Err(format!(
            "IPC line exceeds maximum size ({} > {} bytes)",
            line.len(),
            MAX_IPC_LINE_BYTES
        ));
    }

    let trimmed = trim_line_end(line);
    if trimmed.is_empty() {
        return Err("Empty IPC JSON line".to_string());
    }

    serde_json::from_slice(trimmed).map_err(|err| format!("JSON decode error: {err}"))
}

fn trim_line_end(mut bytes: &[u8]) -> &[u8] {
    while let Some(&last) = bytes.last() {
        if last == b'\n' || last == b'\r' {
            bytes = &bytes[..bytes.len() - 1];
        } else {
            break;
        }
    }
    bytes
}
