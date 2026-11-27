use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use touchpad_protocol::{ScreenEdge, DEFAULT_PORT};
use uuid::Uuid;

pub const CONFIG_VERSION: u32 = 1;

pub const DEFAULT_SENSITIVITY: f32 = 1.6;
pub const MIN_SENSITIVITY: f32 = 0.5;
pub const MAX_SENSITIVITY: f32 = 4.0;

pub const DEFAULT_SCROLL_SENSITIVITY: f32 = 1.0;
pub const MIN_SCROLL_SENSITIVITY: f32 = 0.1;
pub const MAX_SCROLL_SENSITIVITY: f32 = 5.0;

pub const DEFAULT_EDGE_OVERSHOOT_PX: u16 = 28;
pub const MAX_EDGE_OVERSHOOT_PX: u16 = 500;

pub const DEFAULT_EDGE_COOLDOWN_MS: u64 = 350;
pub const MIN_EDGE_COOLDOWN_MS: u64 = 50;
pub const MAX_EDGE_COOLDOWN_MS: u64 = 5000;

pub const DEFAULT_HOLD_DURATION_MS: u64 = 260;
pub const MIN_HOLD_DURATION_MS: u64 = 120;
pub const MAX_HOLD_DURATION_MS: u64 = 1500;

pub const DEFAULT_LOCAL_SENSITIVITY: f32 = 1.0;
pub const MIN_LOCAL_SENSITIVITY: f32 = 0.3;
pub const MAX_LOCAL_SENSITIVITY: f32 = 3.0;

/// Trait implemented by configuration models that normalize and validate their fields.
pub trait Sanitizable {
    fn sanitize(&mut self);
}

/// A paired Android/client device permitted to control the desktop server.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AllowedDevice {
    pub client_id: String,
    pub client_name: String,
    pub last_ip: String,
    pub approved_at_ms: u64,
    pub last_seen_ms: u64,
}

/// Persisted desktop server / Windows service configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopConfig {
    pub version: u32,
    pub computer_name: String,
    pub port: u16,
    pub sensitivity: f32,
    pub scroll_sensitivity: f32,
    pub require_pairing_approval: bool,
    pub edge_overshoot_px: u16,
    pub edge_cooldown_ms: u64,
    pub start_minimized_to_tray: bool,
    pub allowed_devices: Vec<AllowedDevice>,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            computer_name: default_computer_name(),
            port: DEFAULT_PORT,
            sensitivity: DEFAULT_SENSITIVITY,
            scroll_sensitivity: DEFAULT_SCROLL_SENSITIVITY,
            require_pairing_approval: true,
            edge_overshoot_px: DEFAULT_EDGE_OVERSHOOT_PX,
            edge_cooldown_ms: DEFAULT_EDGE_COOLDOWN_MS,
            start_minimized_to_tray: false,
            allowed_devices: Vec::new(),
        }
    }
}

impl DesktopConfig {
    /// Normalizes fields, replaces invalid floats/ports, clamps bounds, and deduplicates devices.
    pub fn sanitize(&mut self) {
        if self.version == 0 {
            self.version = CONFIG_VERSION;
        }

        self.computer_name = self.computer_name.trim().to_string();
        if self.computer_name.is_empty() {
            self.computer_name = default_computer_name();
        }

        if self.port == 0 {
            self.port = DEFAULT_PORT;
        }

        self.sensitivity = sanitize_f32(
            self.sensitivity,
            DEFAULT_SENSITIVITY,
            MIN_SENSITIVITY,
            MAX_SENSITIVITY,
        );
        self.scroll_sensitivity = sanitize_f32(
            self.scroll_sensitivity,
            DEFAULT_SCROLL_SENSITIVITY,
            MIN_SCROLL_SENSITIVITY,
            MAX_SCROLL_SENSITIVITY,
        );
        self.edge_overshoot_px = self.edge_overshoot_px.min(MAX_EDGE_OVERSHOOT_PX);
        self.edge_cooldown_ms = self
            .edge_cooldown_ms
            .clamp(MIN_EDGE_COOLDOWN_MS, MAX_EDGE_COOLDOWN_MS);

        let mut deduped: Vec<AllowedDevice> = Vec::with_capacity(self.allowed_devices.len());
        let mut index_by_key: HashMap<DeviceKey, usize> = HashMap::new();

        for mut device in self.allowed_devices.drain(..) {
            trim_allowed_device(&mut device);
            let Some(key) = device_key(&device) else {
                continue;
            };

            if let Some(&idx) = index_by_key.get(&key) {
                merge_allowed_device(&mut deduped[idx], device);
            } else {
                index_by_key.insert(key, deduped.len());
                deduped.push(device);
            }
        }

        self.allowed_devices = deduped;
    }
}

impl Sanitizable for DesktopConfig {
    fn sanitize(&mut self) {
        DesktopConfig::sanitize(self);
    }
}

/// A desktop target saved in the Android application's configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedComputer {
    pub id: String,
    pub name: String,
    pub ip_address: String,
    pub port: u16,
    pub connection_method_id: String,
    pub last_os: Option<String>,
    pub last_privilege: Option<String>,
}

impl Default for SavedComputer {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            ip_address: String::new(),
            port: DEFAULT_PORT,
            connection_method_id: String::new(),
            last_os: None,
            last_privilege: None,
        }
    }
}

/// Spatial edge adjacency link between two saved computers for Multi-PC switching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EdgeLink {
    pub from_computer_id: String,
    pub from_edge: ScreenEdge,
    pub to_computer_id: String,
    pub to_edge: ScreenEdge,
}

impl Default for EdgeLink {
    fn default() -> Self {
        Self {
            from_computer_id: String::new(),
            from_edge: ScreenEdge::Right,
            to_computer_id: String::new(),
            to_edge: ScreenEdge::Left,
        }
    }
}

/// Persisted Android client configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AndroidConfig {
    pub version: u32,
    pub client_id: String,
    pub computers: Vec<SavedComputer>,
    pub edge_links: Vec<EdgeLink>,
    pub multi_pc_enabled: bool,
    pub hold_duration_ms: u64,
    pub haptics_enabled: bool,
    pub default_landscape: bool,
    pub edge_overshoot_px: u16,
    pub local_sensitivity: f32,
}

impl Default for AndroidConfig {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            client_id: Uuid::new_v4().to_string(),
            computers: Vec::new(),
            edge_links: Vec::new(),
            multi_pc_enabled: false,
            hold_duration_ms: DEFAULT_HOLD_DURATION_MS,
            haptics_enabled: true,
            default_landscape: true,
            edge_overshoot_px: DEFAULT_EDGE_OVERSHOOT_PX,
            local_sensitivity: DEFAULT_LOCAL_SENSITIVITY,
        }
    }
}

impl AndroidConfig {
    /// Normalizes fields, ensures a UUID v4 client ID, deduplicates computers by `ip:port`,
    /// and removes dangling or self-referential edge links.
    pub fn sanitize(&mut self) {
        if self.version == 0 {
            self.version = CONFIG_VERSION;
        }

        self.client_id = self.client_id.trim().to_string();
        if self.client_id.is_empty() {
            self.client_id = Uuid::new_v4().to_string();
        }

        self.hold_duration_ms = self
            .hold_duration_ms
            .clamp(MIN_HOLD_DURATION_MS, MAX_HOLD_DURATION_MS);
        self.edge_overshoot_px = self.edge_overshoot_px.min(MAX_EDGE_OVERSHOOT_PX);
        self.local_sensitivity = sanitize_f32(
            self.local_sensitivity,
            DEFAULT_LOCAL_SENSITIVITY,
            MIN_LOCAL_SENSITIVITY,
            MAX_LOCAL_SENSITIVITY,
        );

        let mut deduped_computers: Vec<SavedComputer> = Vec::with_capacity(self.computers.len());
        let mut index_by_endpoint: HashMap<(String, u16), usize> = HashMap::new();
        let mut seen_ids: HashSet<String> = HashSet::new();

        for mut computer in self.computers.drain(..) {
            computer.id = computer.id.trim().to_string();
            computer.name = computer.name.trim().to_string();
            computer.ip_address = computer.ip_address.trim().to_string();
            computer.connection_method_id = computer.connection_method_id.trim().to_string();
            computer.last_os = sanitize_optional_string(computer.last_os.take());
            computer.last_privilege = sanitize_optional_string(computer.last_privilege.take());

            if computer.ip_address.is_empty() {
                continue;
            }
            if computer.port == 0 {
                computer.port = DEFAULT_PORT;
            }

            let endpoint_key = (computer.ip_address.to_ascii_lowercase(), computer.port);
            if let Some(&idx) = index_by_endpoint.get(&endpoint_key) {
                let existing = &mut deduped_computers[idx];
                if existing.name.is_empty() && !computer.name.is_empty() {
                    existing.name = computer.name;
                }
                if existing.connection_method_id.is_empty()
                    && !computer.connection_method_id.is_empty()
                {
                    existing.connection_method_id = computer.connection_method_id;
                }
                if existing.last_os.is_none() {
                    existing.last_os = computer.last_os;
                }
                if existing.last_privilege.is_none() {
                    existing.last_privilege = computer.last_privilege;
                }
                continue;
            }

            if computer.id.is_empty() || seen_ids.contains(&computer.id) {
                computer.id = Uuid::new_v4().to_string();
            }
            seen_ids.insert(computer.id.clone());
            index_by_endpoint.insert(endpoint_key, deduped_computers.len());
            deduped_computers.push(computer);
        }

        self.computers = deduped_computers;

        let mut deduped_links: Vec<EdgeLink> = Vec::with_capacity(self.edge_links.len());
        let mut seen_links: HashSet<(String, ScreenEdge, String, ScreenEdge)> = HashSet::new();

        for mut link in self.edge_links.drain(..) {
            link.from_computer_id = link.from_computer_id.trim().to_string();
            link.to_computer_id = link.to_computer_id.trim().to_string();

            if link.from_computer_id.is_empty()
                || link.to_computer_id.is_empty()
                || link.from_computer_id == link.to_computer_id
                || !seen_ids.contains(&link.from_computer_id)
                || !seen_ids.contains(&link.to_computer_id)
            {
                continue;
            }

            let key = (
                link.from_computer_id.clone(),
                link.from_edge,
                link.to_computer_id.clone(),
                link.to_edge,
            );
            if seen_links.insert(key) {
                deduped_links.push(link);
            }
        }

        self.edge_links = deduped_links;
    }
}

impl Sanitizable for AndroidConfig {
    fn sanitize(&mut self) {
        AndroidConfig::sanitize(self);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum DeviceKey {
    ClientId(String),
    LastIp(String),
}

pub(crate) fn trim_allowed_device(device: &mut AllowedDevice) {
    device.client_id = device.client_id.trim().to_string();
    device.client_name = device.client_name.trim().to_string();
    device.last_ip = device.last_ip.trim().to_string();
}

pub(crate) fn device_key(device: &AllowedDevice) -> Option<DeviceKey> {
    let client_id = device.client_id.trim();
    if !client_id.is_empty() {
        return Some(DeviceKey::ClientId(client_id.to_string()));
    }
    let last_ip = device.last_ip.trim();
    if !last_ip.is_empty() {
        return Some(DeviceKey::LastIp(last_ip.to_string()));
    }
    None
}

/// Merges `incoming` into `existing`, keeping whichever entry has higher `(last_seen_ms, approved_at_ms)`
/// (preferring `existing` on ties) and backfilling missing metadata from the other entry.
pub(crate) fn merge_allowed_device(existing: &mut AllowedDevice, incoming: AllowedDevice) {
    let incoming_is_newer =
        (incoming.last_seen_ms, incoming.approved_at_ms) > (existing.last_seen_ms, existing.approved_at_ms);

    let (mut primary, secondary) = if incoming_is_newer {
        (incoming, std::mem::take(existing))
    } else {
        (std::mem::take(existing), incoming)
    };

    if primary.client_id.is_empty() && !secondary.client_id.is_empty() {
        primary.client_id = secondary.client_id;
    }
    if primary.client_name.is_empty() && !secondary.client_name.is_empty() {
        primary.client_name = secondary.client_name;
    }
    if primary.last_ip.is_empty() && !secondary.last_ip.is_empty() {
        primary.last_ip = secondary.last_ip;
    }
    if primary.approved_at_ms == 0 && secondary.approved_at_ms != 0 {
        primary.approved_at_ms = secondary.approved_at_ms;
    }

    *existing = primary;
}

fn sanitize_f32(value: f32, default: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}

fn sanitize_optional_string(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn default_computer_name() -> String {
    for var in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(val) = std::env::var(var) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return trimmed.to_string();
            }
        }
    }
    "Desktop".to_string()
}
