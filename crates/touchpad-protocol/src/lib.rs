//! Wire protocol v1 and v2 for TouchPad Remote.

use serde::{Deserialize, Serialize};
use std::str::FromStr;

pub const DEFAULT_PORT: u16 = 45454;

const DEFAULT_CLIENT_ID: &str = "android-default";
const DEFAULT_CLIENT_NAME: &str = "AndroidTouchpad";
const DEFAULT_SERVER_NAME: &str = "Unknown";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

impl MouseButton {
    fn wire_name(&self) -> &'static str {
        match self {
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
            Self::Middle => "MIDDLE",
        }
    }
}

impl FromStr for MouseButton {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let token = clean_token(raw).ok_or(())?;
        [Self::Left, Self::Right, Self::Middle]
            .into_iter()
            .find(|value| value.wire_name().eq_ignore_ascii_case(token))
            .ok_or(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PairingStatus {
    Pending,
    Approved,
    Rejected,
}

impl PairingStatus {
    fn wire_name(&self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Approved => "APPROVED",
            Self::Rejected => "REJECTED",
        }
    }
}

impl FromStr for PairingStatus {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let token = clean_token(raw).ok_or(())?;
        [Self::Pending, Self::Approved, Self::Rejected]
            .into_iter()
            .find(|value| value.wire_name().eq_ignore_ascii_case(token))
            .ok_or(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConnectionMethod {
    NetworkLan,
    Bluetooth,
    UsbAdb,
}

impl ConnectionMethod {
    pub fn id(&self) -> &'static str {
        match self {
            Self::NetworkLan => "NETWORK_LAN",
            Self::Bluetooth => "BLUETOOTH",
            Self::UsbAdb => "USB_ADB",
        }
    }

    pub fn from_id(id: &str) -> Self {
        id.trim().parse().unwrap_or(Self::NetworkLan)
    }

    pub fn is_available_now(&self) -> bool {
        matches!(self, Self::NetworkLan)
    }
}

impl FromStr for ConnectionMethod {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let token = clean_token(raw).ok_or(())?;
        [Self::NetworkLan, Self::Bluetooth, Self::UsbAdb]
            .into_iter()
            .find(|value| value.id().eq_ignore_ascii_case(token))
            .ok_or(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ScreenEdge {
    Left,
    Right,
    Top,
    Bottom,
}

impl ScreenEdge {
    fn wire_name(&self) -> &'static str {
        match self {
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
            Self::Top => "TOP",
            Self::Bottom => "BOTTOM",
        }
    }

    pub fn opposite(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
        }
    }
}

impl FromStr for ScreenEdge {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let token = clean_token(raw).ok_or(())?;
        [Self::Left, Self::Right, Self::Top, Self::Bottom]
            .into_iter()
            .find(|value| value.wire_name().eq_ignore_ascii_case(token))
            .ok_or(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HostOs {
    Windows,
    Linux,
}

impl HostOs {
    fn wire_name(&self) -> &'static str {
        match self {
            Self::Windows => "WINDOWS",
            Self::Linux => "LINUX",
        }
    }
}

impl FromStr for HostOs {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let token = clean_token(raw).ok_or(())?;
        [Self::Windows, Self::Linux]
            .into_iter()
            .find(|value| value.wire_name().eq_ignore_ascii_case(token))
            .ok_or(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PrivilegeMode {
    User,
    Admin,
    System,
    Uinput,
    Xtest,
}

impl PrivilegeMode {
    fn wire_name(&self) -> &'static str {
        match self {
            Self::User => "USER",
            Self::Admin => "ADMIN",
            Self::System => "SYSTEM",
            Self::Uinput => "UINPUT",
            Self::Xtest => "XTEST",
        }
    }
}

impl FromStr for PrivilegeMode {
    type Err = ();

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let token = clean_token(raw).ok_or(())?;
        [
            Self::User,
            Self::Admin,
            Self::System,
            Self::Uinput,
            Self::Xtest,
        ]
        .into_iter()
        .find(|value| value.wire_name().eq_ignore_ascii_case(token))
        .ok_or(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TouchpadPacket {
    Move {
        dx: f32,
        dy: f32,
    },
    Click {
        button: MouseButton,
    },
    ButtonState {
        button: MouseButton,
        pressed: bool,
    },
    Scroll {
        delta_y: f32,
    },
    DiscoverRequest {
        client_name: String,
    },
    DiscoverResponse {
        computer_name: String,
        ip_address: String,
        port: u16,
        method: ConnectionMethod,
    },
    Ping {
        timestamp: i64,
    },
    Pong {
        timestamp: i64,
        server_name: String,
    },
    PairRequest {
        client_id: String,
        client_name: String,
    },
    PairResponse {
        status: PairingStatus,
        server_name: String,
    },
    EdgeConfig {
        left: bool,
        right: bool,
        top: bool,
        bottom: bool,
        overshoot_px: u16,
    },
    EdgeHit {
        edge: ScreenEdge,
        ratio: f32,
    },
    CursorEnter {
        edge: ScreenEdge,
        ratio: f32,
    },
    CursorLeave {
        edge: ScreenEdge,
    },
    HostInfo {
        os: HostOs,
        width: u32,
        height: u32,
        privilege: PrivilegeMode,
    },
}

impl TouchpadPacket {
    pub fn encode(&self) -> String {
        match self {
            Self::Move { dx, dy } => format!("MOVE|{}|{}", format_float(*dx), format_float(*dy)),
            Self::Click { button } => format!("CLICK|{}", button.wire_name()),
            Self::ButtonState { button, pressed } => {
                format!("BUTTON_STATE|{}|{}", button.wire_name(), pressed)
            }
            Self::Scroll { delta_y } => format!("SCROLL|{}", format_float(*delta_y)),
            Self::DiscoverRequest { client_name } => format!(
                "DISCOVER_REQ|{}",
                sanitize_text(client_name, DEFAULT_CLIENT_NAME)
            ),
            Self::DiscoverResponse {
                computer_name,
                ip_address,
                port,
                method,
            } => format!(
                "DISCOVER_RESP|{}|{}|{}|{}",
                sanitize_text(computer_name, DEFAULT_SERVER_NAME),
                sanitize_compact_token(ip_address),
                port,
                method.id()
            ),
            Self::Ping { timestamp } => format!("PING|{}", timestamp),
            Self::Pong {
                timestamp,
                server_name,
            } => format!(
                "PONG|{}|{}",
                timestamp,
                sanitize_text(server_name, DEFAULT_SERVER_NAME)
            ),
            Self::PairRequest {
                client_id,
                client_name,
            } => format!(
                "PAIR_REQ|{}|{}",
                text_or_default(sanitize_compact_token(client_id), DEFAULT_CLIENT_ID),
                sanitize_text(client_name, DEFAULT_CLIENT_NAME)
            ),
            Self::PairResponse {
                status,
                server_name,
            } => format!(
                "PAIR_RESP|{}|{}",
                status.wire_name(),
                sanitize_text(server_name, DEFAULT_SERVER_NAME)
            ),
            Self::EdgeConfig {
                left,
                right,
                top,
                bottom,
                overshoot_px,
            } => format!(
                "EDGE_CONFIG|{}|{}|{}|{}|{}",
                left, right, top, bottom, overshoot_px
            ),
            Self::EdgeHit { edge, ratio } => format!(
                "EDGE_HIT|{}|{}",
                edge.wire_name(),
                format_float(clamp_ratio(*ratio))
            ),
            Self::CursorEnter { edge, ratio } => format!(
                "CURSOR_ENTER|{}|{}",
                edge.wire_name(),
                format_float(clamp_ratio(*ratio))
            ),
            Self::CursorLeave { edge } => format!("CURSOR_LEAVE|{}", edge.wire_name()),
            Self::HostInfo {
                os,
                width,
                height,
                privilege,
            } => format!(
                "HOST_INFO|{}|{}|{}|{}",
                os.wire_name(),
                width,
                height,
                privilege.wire_name()
            ),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.encode().into_bytes()
    }

    pub fn decode(raw: &str) -> Option<Self> {
        // Six fields is the largest valid packet. Stop at the first extra field.
        let mut fields = [""; 6];
        let mut count = 0;
        for field in raw.trim().split('|') {
            *fields.get_mut(count)? = field;
            count += 1;
        }

        Some(match &fields[..count] {
            ["MOVE", dx, dy] => Self::Move {
                dx: parse_float(dx)?,
                dy: parse_float(dy)?,
            },
            ["CLICK", button] => Self::Click {
                button: button.parse().ok()?,
            },
            ["BUTTON_STATE", button, pressed] => Self::ButtonState {
                button: button.parse().ok()?,
                pressed: parse_bool(pressed)?,
            },
            ["SCROLL", delta_y] => Self::Scroll {
                delta_y: parse_float(delta_y)?,
            },
            ["DISCOVER_REQ", client_name] => Self::DiscoverRequest {
                client_name: sanitize_text(client_name, DEFAULT_CLIENT_NAME),
            },
            ["DISCOVER_RESP", computer_name, ip_address, port, method] => Self::DiscoverResponse {
                computer_name: nonempty(sanitize_text(computer_name, ""))?,
                ip_address: nonempty(sanitize_compact_token(ip_address))?,
                port: parse_positive(port)?,
                method: ConnectionMethod::from_id(nonempty_token(method)?),
            },
            ["PING", timestamp] => Self::Ping {
                timestamp: parse_number(timestamp)?,
            },
            ["PONG", timestamp, server_name] => Self::Pong {
                timestamp: parse_number(timestamp)?,
                server_name: nonempty(sanitize_text(server_name, ""))?,
            },
            ["PAIR_REQ", client_id, client_name] => Self::PairRequest {
                client_id: nonempty(sanitize_compact_token(client_id))?,
                client_name: nonempty(sanitize_text(client_name, ""))?,
            },
            ["PAIR_RESP", status, server_name] => Self::PairResponse {
                status: status.parse().ok()?,
                server_name: nonempty(sanitize_text(server_name, ""))?,
            },
            ["EDGE_CONFIG", left, right, top, bottom, overshoot_px] => Self::EdgeConfig {
                left: parse_bool(left)?,
                right: parse_bool(right)?,
                top: parse_bool(top)?,
                bottom: parse_bool(bottom)?,
                overshoot_px: parse_number(overshoot_px)?,
            },
            ["EDGE_HIT", edge, ratio] => Self::EdgeHit {
                edge: edge.parse().ok()?,
                ratio: parse_float(ratio)?.clamp(0.0, 1.0),
            },
            ["CURSOR_ENTER", edge, ratio] => Self::CursorEnter {
                edge: edge.parse().ok()?,
                ratio: parse_float(ratio)?.clamp(0.0, 1.0),
            },
            ["CURSOR_LEAVE", edge] => Self::CursorLeave {
                edge: edge.parse().ok()?,
            },
            ["HOST_INFO", os, width, height, privilege] => Self::HostInfo {
                os: os.parse().ok()?,
                width: parse_positive(width)?,
                height: parse_positive(height)?,
                privilege: privilege.parse().ok()?,
            },
            _ => return None,
        })
    }

    /// Decode the exact UTF-8 datagram slice supplied by the caller.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        Self::decode(std::str::from_utf8(bytes).ok()?)
    }
}

fn clean_token(token: &str) -> Option<&str> {
    (!token.chars().any(char::is_control)).then(|| token.trim())
}

fn nonempty_token(token: &str) -> Option<&str> {
    let token = clean_token(token)?;
    (!token.is_empty()).then_some(token)
}

fn parse_number<T: FromStr>(token: &str) -> Option<T> {
    clean_token(token)?.parse().ok()
}

fn parse_positive<T: FromStr + PartialOrd + From<u8>>(token: &str) -> Option<T> {
    let value = parse_number(token)?;
    (value >= T::from(1)).then_some(value)
}

fn parse_bool(token: &str) -> Option<bool> {
    match clean_token(token)? {
        token if token.eq_ignore_ascii_case("true") => Some(true),
        token if token.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

fn parse_float(token: &str) -> Option<f32> {
    let value: f32 = parse_number(token)?;
    value.is_finite().then(|| normalize_zero(value))
}

fn normalize_zero(value: f32) -> f32 {
    if value == 0.0 {
        0.0
    } else {
        value
    }
}

fn format_float(value: f32) -> String {
    let value = normalize_zero(value);
    let magnitude = value.abs();
    // Match the Java Float.toString notation used by the v1 codec.
    if value.is_finite() && magnitude != 0.0 && !(0.001..10_000_000.0).contains(&magnitude) {
        let scientific = format!("{value:E}");
        if let Some((mantissa, exponent)) = scientific.split_once('E') {
            if !mantissa.contains('.') {
                return format!("{mantissa}.0E{exponent}");
            }
        }
        return scientific;
    }
    format!("{value:?}")
}

fn clamp_ratio(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 1.0)
    } else {
        value
    }
}

fn sanitize_text(value: &str, fallback: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    let mut pending_space = false;
    for ch in value.chars() {
        match ch {
            '|' | '\n' | '\r' | '\t' | ' ' => pending_space = true,
            ch if ch.is_control() => {}
            ch => {
                if pending_space && !sanitized.is_empty() {
                    sanitized.push(' ');
                }
                sanitized.push(ch);
                pending_space = false;
            }
        }
    }
    text_or_default(sanitized.trim_end().to_owned(), fallback)
}

fn sanitize_compact_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| *ch != '|' && !ch.is_whitespace() && !ch.is_control())
        .collect()
}

fn nonempty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

fn text_or_default(value: String, fallback: &str) -> String {
    nonempty(value).unwrap_or_else(|| fallback.to_owned())
}
