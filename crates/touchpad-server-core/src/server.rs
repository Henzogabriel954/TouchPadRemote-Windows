use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use touchpad_protocol::{ConnectionMethod, PairingStatus, TouchpadPacket};
use touchpad_storage::{AllowedDevice, ConfigStore, DesktopConfig, StorageError};

use crate::edge::{EdgeConfig, EdgeDetector};
use crate::input::{desktop_limits, InputBackend, MouseController};
use crate::network::{get_local_ip_addresses, resolve_best_local_ip_for};

const REJECT_COOLDOWN: Duration = Duration::from_millis(3500);
const REPROMPT_COOLDOWN: Duration = Duration::from_secs(15);

#[derive(Debug, Error)]
pub enum ServerError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingClientInfo {
    pub client_id: String,
    pub client_name: String,
    pub ip_address: IpAddr,
    pub requested_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerSnapshot {
    pub running: bool,
    pub actual_port: u16,
    pub packet_count: u64,
    pub last_client_ip: Option<IpAddr>,
    pub pending_pairings: Vec<PairingClientInfo>,
    pub approved_clients: Vec<PairingClientInfo>,
    pub config: DesktopConfig,
}

#[derive(Debug, Clone)]
pub enum ServerEvent {
    StateChanged {
        running: bool,
        port: u16,
    },
    PacketReceived {
        packet: TouchpadPacket,
        client_ip: IpAddr,
        total_packets: u64,
    },
    DiscoveryRequested {
        client_name: String,
        client_ip: IpAddr,
        responded_ip: String,
    },
    PingReceived {
        client_ip: IpAddr,
    },
    PairingRequested(PairingClientInfo),
    PairingStateChanged,
    Error(String),
}

pub type EventCallback = Arc<dyn Fn(ServerEvent) + Send + Sync + 'static>;

struct State {
    config: DesktopConfig,
    running: bool,
    actual_port: u16,
    packet_count: u64,
    last_client_ip: Option<IpAddr>,
    pending: HashMap<IpAddr, PairingClientInfo>,
    approved: HashMap<IpAddr, PairingClientInfo>,
    rejected_ips: HashMap<IpAddr, Instant>,
    rejected_ids: HashMap<String, Instant>,
    mouse: MouseController,
    edge: EdgeDetector,
    host_info: Option<TouchpadPacket>,
    peers: HashMap<IpAddr, SocketAddr>,
}

impl State {
    fn can_control(&self, ip: IpAddr) -> bool {
        // A control packet carries no client ID; the persisted or paired IP is its association.
        let rejected = self
            .rejected_ips
            .get(&ip)
            .is_some_and(|at| at.elapsed() < REJECT_COOLDOWN);
        !rejected && (!self.config.require_pairing_approval || self.approved.contains_key(&ip))
    }

    fn release_input(&mut self) {
        self.mouse.release_all_buttons();
        self.mouse.reset_accumulators();
        self.edge.reset();
    }

    fn apply_config(&mut self, config: DesktopConfig) {
        self.mouse
            .update_sensitivity(config.sensitivity, config.scroll_sensitivity);
        self.edge.set_cooldown_ms(config.edge_cooldown_ms);
        if config.edge_overshoot_px != self.config.edge_overshoot_px {
            self.edge.update_config(EdgeConfig {
                overshoot_px: config.edge_overshoot_px,
                ..self.edge.config()
            });
        }
        if config.require_pairing_approval != self.config.require_pairing_approval {
            self.release_input();
        }
        self.config = config;
    }

    fn restore_approved(&mut self) {
        self.approved.clear();
        let mut devices: Vec<_> = self.config.allowed_devices.iter().collect();
        devices.sort_by_key(|device| device.last_seen_ms);
        for device in devices {
            if let Ok(ip_address) = device.last_ip.parse() {
                self.approved.insert(
                    ip_address,
                    PairingClientInfo {
                        client_id: device.client_id.clone(),
                        client_name: device.client_name.clone(),
                        ip_address,
                        requested_at_ms: device.approved_at_ms,
                    },
                );
            }
        }
    }
}

struct Shared {
    backend: Arc<dyn InputBackend>,
    store: Arc<ConfigStore<DesktopConfig>>,
    state: Mutex<State>,
    callback: Option<EventCallback>,
}

pub struct TouchpadServer {
    shared: Arc<Shared>,
    requested_port: u16,
    worker: Option<JoinHandle<()>>,
}

impl TouchpadServer {
    pub fn new(
        backend: Arc<dyn InputBackend>,
        store: impl Into<Arc<ConfigStore<DesktopConfig>>>,
    ) -> Self {
        let store = store.into();
        let port = store.get().port;
        Self::with_port(backend, store, port)
    }

    /// Overrides the stored bind port, including port 0 for an OS-assigned port.
    pub fn with_port(
        backend: Arc<dyn InputBackend>,
        store: impl Into<Arc<ConfigStore<DesktopConfig>>>,
        port: u16,
    ) -> Self {
        let store = store.into();
        let config = store.get();
        let mouse = MouseController::new(
            Arc::clone(&backend),
            config.sensitivity,
            config.scroll_sensitivity,
        );
        let edge = EdgeDetector::new(
            EdgeConfig {
                overshoot_px: config.edge_overshoot_px,
                ..EdgeConfig::default()
            },
            config.edge_cooldown_ms,
        );
        let mut state = State {
            config,
            running: false,
            actual_port: port,
            packet_count: 0,
            last_client_ip: None,
            pending: HashMap::new(),
            approved: HashMap::new(),
            rejected_ips: HashMap::new(),
            rejected_ids: HashMap::new(),
            mouse,
            edge,
            host_info: None,
            peers: HashMap::new(),
        };
        state.restore_approved();
        Self {
            shared: Arc::new(Shared {
                backend,
                store,
                state: Mutex::new(state),
                callback: None,
            }),
            requested_port: port,
            worker: None,
        }
    }

    pub fn with_event_callback(mut self, callback: EventCallback) -> Self {
        Arc::get_mut(&mut self.shared)
            .expect("configure event callback before starting")
            .callback = Some(callback);
        self
    }

    pub fn start(&mut self) -> Result<(), ServerError> {
        if self.is_running() {
            return Ok(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        match self.bind_and_start() {
            Ok(port) => {
                self.shared.emit(ServerEvent::StateChanged {
                    running: true,
                    port,
                });
                Ok(())
            }
            Err(error) => {
                self.shared.emit(ServerEvent::Error(error.to_string()));
                Err(error)
            }
        }
    }

    fn bind_and_start(&mut self) -> Result<u16, ServerError> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, self.requested_port))?;
        socket.set_read_timeout(Some(Duration::from_millis(50)))?;
        let port = socket.local_addr()?.port();
        let shared = Arc::clone(&self.shared);
        let mut state = self.shared.state.lock().unwrap();
        let worker = thread::Builder::new()
            .name(format!("touchpad-udp-{port}"))
            .spawn(move || shared.receive(socket))?;
        state.apply_config(self.shared.store.get());
        state.restore_approved();
        state.actual_port = port;
        state.running = true;
        self.worker = Some(worker);
        Ok(port)
    }

    pub fn stop(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        let (running, port) = {
            let mut state = self.shared.state.lock().unwrap();
            let running = state.running;
            state.running = false;
            state.pending.clear();
            state.approved.clear();
            state.rejected_ips.clear();
            state.rejected_ids.clear();
            state.peers.clear();
            state.host_info = None;
            state.release_input();
            (running, state.actual_port)
        };
        if worker.thread().id() != thread::current().id() {
            let _ = worker.join();
        }
        if running {
            self.shared.emit(ServerEvent::PairingStateChanged);
            self.shared.emit(ServerEvent::StateChanged {
                running: false,
                port,
            });
        }
    }

    pub fn actual_port(&self) -> u16 {
        self.shared.state.lock().unwrap().actual_port
    }

    pub fn is_running(&self) -> bool {
        self.shared.state.lock().unwrap().running
    }

    pub fn snapshot(&self) -> ServerSnapshot {
        let state = self.shared.state.lock().unwrap();
        let sorted = |clients: &HashMap<IpAddr, PairingClientInfo>| {
            let mut clients: Vec<_> = clients.values().cloned().collect();
            clients.sort_by_key(|entry| (entry.requested_at_ms, entry.ip_address));
            clients
        };
        ServerSnapshot {
            running: state.running,
            actual_port: state.actual_port,
            packet_count: state.packet_count,
            last_client_ip: state.last_client_ip,
            pending_pairings: sorted(&state.pending),
            approved_clients: sorted(&state.approved),
            config: state.config.clone(),
        }
    }

    pub fn update_settings(
        &self,
        sensitivity: f32,
        scroll_sensitivity: f32,
        edge_cooldown_ms: u64,
        require_pairing_approval: bool,
    ) -> Result<DesktopConfig, ServerError> {
        let mut state = self.shared.state.lock().unwrap();
        let config = self.shared.store.update(|config| {
            config.sensitivity = sensitivity;
            config.scroll_sensitivity = scroll_sensitivity;
            config.edge_cooldown_ms = edge_cooldown_ms;
            config.require_pairing_approval = require_pairing_approval;
        })?;
        state.apply_config(config.clone());
        Ok(config)
    }

    pub fn approve_pairing(&self, ip: IpAddr) -> Result<(), ServerError> {
        {
            let mut state = self.shared.state.lock().unwrap();
            let client = state
                .pending
                .get(&ip)
                .or_else(|| state.approved.get(&ip))
                .cloned()
                .unwrap_or_else(|| PairingClientInfo {
                    client_id: String::new(),
                    client_name: ip.to_string(),
                    ip_address: ip,
                    requested_at_ms: now_ms(),
                });
            self.shared.approve(&mut state, client)?;
        }
        self.shared.emit(ServerEvent::PairingStateChanged);
        Ok(())
    }

    /// Revokes by client ID, and also accepts the IP of an active or pending pairing.
    pub fn revoke_device(&self, client_id_or_ip: &str) -> Result<(), ServerError> {
        {
            let mut state = self.shared.state.lock().unwrap();
            let matches = |client: &PairingClientInfo| {
                (!client.client_id.is_empty() && client.client_id == client_id_or_ip)
                    || client.ip_address.to_string() == client_id_or_ip
            };
            let removed: Vec<_> = state
                .approved
                .values()
                .chain(state.pending.values())
                .filter(|client| matches(client))
                .cloned()
                .collect();
            let mut rejected_ids: Vec<_> = state
                .config
                .allowed_devices
                .iter()
                .filter(|device| {
                    device.client_id == client_id_or_ip || device.last_ip == client_id_or_ip
                })
                .map(|device| device.client_id.clone())
                .filter(|id| !id.is_empty())
                .collect();
            rejected_ids.extend(
                removed
                    .iter()
                    .map(|client| client.client_id.clone())
                    .filter(|id| !id.is_empty()),
            );
            let config = self.shared.store.update(|config| {
                config.allowed_devices.retain(|device| {
                    device.last_ip != client_id_or_ip && !rejected_ids.contains(&device.client_id)
                });
            })?;
            state.config = config;
            state.pending.retain(|_, client| !matches(client));
            state.approved.retain(|_, client| !matches(client));
            let now = Instant::now();
            for client in removed {
                state.rejected_ips.insert(client.ip_address, now);
            }
            if let Ok(ip) = client_id_or_ip.parse() {
                state.rejected_ips.insert(ip, now);
            }
            for id in rejected_ids {
                state.rejected_ids.insert(id, now);
            }
            state.release_input();
        }
        self.shared.emit(ServerEvent::PairingStateChanged);
        Ok(())
    }

    pub fn reject_pairing(&self, ip: IpAddr) -> Result<(), ServerError> {
        self.revoke_device(&ip.to_string())
    }
}

impl Drop for TouchpadServer {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Shared {
    fn emit(&self, event: ServerEvent) {
        if let Some(callback) = &self.callback {
            let _ = catch_unwind(AssertUnwindSafe(|| callback(event)));
        }
    }

    fn receive(&self, socket: UdpSocket) {
        let mut buffer = [0; 2049];
        while self.state.lock().unwrap().running {
            match socket.recv_from(&mut buffer) {
                Ok((size, from)) if size <= 2048 => {
                    if let Some(packet) = TouchpadPacket::from_bytes(&buffer[..size]) {
                        for event in self.handle(&socket, packet, from) {
                            self.emit(event);
                        }
                    }
                }
                Ok(_) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::Interrupted
                            | io::ErrorKind::ConnectionReset
                    ) || error.raw_os_error() == Some(10040) => {}
                Err(error) => {
                    let port = {
                        let mut state = self.state.lock().unwrap();
                        if !state.running {
                            return;
                        }
                        state.running = false;
                        state.release_input();
                        state.actual_port
                    };
                    drop(socket);
                    self.emit(ServerEvent::Error(error.to_string()));
                    self.emit(ServerEvent::StateChanged {
                        running: false,
                        port,
                    });
                    return;
                }
            }
        }
    }

    fn handle(
        &self,
        socket: &UdpSocket,
        packet: TouchpadPacket,
        from: SocketAddr,
    ) -> Vec<ServerEvent> {
        let mut state = self.state.lock().unwrap();
        if !state.running {
            return Vec::new();
        }
        let ip = from.ip();
        let name = state.config.computer_name.clone();
        let mut events = Vec::new();
        let mut force_host_info = false;
        match packet {
            TouchpadPacket::DiscoverRequest { client_name } => {
                let responded_ip = resolve_best_local_ip_for(ip)
                    .or_else(|| get_local_ip_addresses().into_iter().next())
                    .unwrap_or_else(|| Ipv4Addr::LOCALHOST.to_string());
                send(
                    socket,
                    from,
                    TouchpadPacket::DiscoverResponse {
                        computer_name: name,
                        ip_address: responded_ip.clone(),
                        port: state.actual_port,
                        method: ConnectionMethod::NetworkLan,
                    },
                );
                events.push(ServerEvent::DiscoveryRequested {
                    client_name,
                    client_ip: ip,
                    responded_ip,
                });
                return events;
            }
            TouchpadPacket::Ping { timestamp } => {
                send(
                    socket,
                    from,
                    TouchpadPacket::Pong {
                        timestamp,
                        server_name: name,
                    },
                );
                events.push(ServerEvent::PingReceived { client_ip: ip });
                force_host_info = true;
            }
            TouchpadPacket::PairRequest {
                client_id,
                client_name,
            } => {
                let result = self.pair(&mut state, ip, client_id, client_name, &mut events);
                let status = match result {
                    Ok(status) => status,
                    Err(error) => {
                        events.push(ServerEvent::Error(error.to_string()));
                        PairingStatus::Rejected
                    }
                };
                send(
                    socket,
                    from,
                    TouchpadPacket::PairResponse {
                        status,
                        server_name: name,
                    },
                );
                force_host_info = true;
            }
            TouchpadPacket::HostInfo { .. } => force_host_info = true,
            packet @ (TouchpadPacket::Move { .. }
            | TouchpadPacket::Click { .. }
            | TouchpadPacket::ButtonState { .. }
            | TouchpadPacket::Scroll { .. }
            | TouchpadPacket::CursorEnter { .. }
            | TouchpadPacket::CursorLeave { .. }
            | TouchpadPacket::EdgeConfig { .. }) => {
                if !state.can_control(ip) {
                    return events;
                }
                self.control(&mut state, socket, from, &packet);
                self.record_last_seen(&mut state, ip, &mut events);
                state.last_client_ip = Some(ip);
                state.packet_count += 1;
                events.push(ServerEvent::PacketReceived {
                    packet,
                    client_ip: ip,
                    total_packets: state.packet_count,
                });
            }
            _ => return events,
        }

        let previous = state.peers.insert(ip, from);
        let [left, top, right, bottom] = desktop_limits(&self.backend.monitor_bounds());
        let host_info = TouchpadPacket::HostInfo {
            os: self.backend.os_kind(),
            privilege: self.backend.privilege_mode(),
            width: (i64::from(right) - i64::from(left) + 1).min(i64::from(u32::MAX)) as u32,
            height: (i64::from(bottom) - i64::from(top) + 1).min(i64::from(u32::MAX)) as u32,
        };
        if state.host_info.as_ref() != Some(&host_info) {
            for &peer in state.peers.values() {
                send(socket, peer, host_info.clone());
            }
            state.host_info = Some(host_info);
        } else if force_host_info || previous != Some(from) {
            send(socket, from, host_info);
        }
        events
    }

    fn pair(
        &self,
        state: &mut State,
        ip: IpAddr,
        id: String,
        name: String,
        events: &mut Vec<ServerEvent>,
    ) -> Result<PairingStatus, ServerError> {
        let now = Instant::now();
        state
            .rejected_ips
            .retain(|_, at| now.duration_since(*at) < REJECT_COOLDOWN);
        state
            .rejected_ids
            .retain(|_, at| now.duration_since(*at) < REJECT_COOLDOWN);
        if state.rejected_ips.contains_key(&ip) || state.rejected_ids.contains_key(&id) {
            return Ok(PairingStatus::Rejected);
        }
        let requested_at_ms = now_ms();
        let client = PairingClientInfo {
            client_id: id,
            client_name: name,
            ip_address: ip,
            requested_at_ms,
        };
        let known = state.config.allowed_devices.iter().any(|device| {
            device.client_id == client.client_id
                || (device.client_id.is_empty() && device.last_ip == ip.to_string())
        });
        if known {
            let changed = state.approved.get(&ip).is_none_or(|existing| {
                existing.client_id != client.client_id || existing.client_name != client.client_name
            });
            self.approve(state, client)?;
            if changed {
                events.push(ServerEvent::PairingStateChanged);
            }
            return Ok(PairingStatus::Approved);
        }

        if state.approved.contains_key(&ip) {
            let ip_string = ip.to_string();
            state.config = self.store.update(|config| {
                for device in config
                    .allowed_devices
                    .iter_mut()
                    .filter(|device| device.last_ip == ip_string)
                {
                    device.last_ip.clear();
                }
            })?;
            state.approved.remove(&ip);
            state.release_input();
            events.push(ServerEvent::PairingStateChanged);
        }
        if !state.config.require_pairing_approval {
            return Ok(PairingStatus::Approved);
        }

        state
            .pending
            .retain(|key, value| *key == ip || value.client_id != client.client_id);
        let previous = state.pending.get(&ip);
        let changed = previous.is_none_or(|entry| {
            entry.client_id != client.client_id || entry.client_name != client.client_name
        });
        if changed
            || previous.is_some_and(|entry| {
                requested_at_ms.saturating_sub(entry.requested_at_ms)
                    >= REPROMPT_COOLDOWN.as_millis() as u64
            })
        {
            state.pending.insert(ip, client.clone());
            events.push(ServerEvent::PairingRequested(client));
            if changed {
                events.push(ServerEvent::PairingStateChanged);
            }
        }
        Ok(PairingStatus::Pending)
    }

    fn approve(
        &self,
        state: &mut State,
        mut client: PairingClientInfo,
    ) -> Result<(), StorageError> {
        let ip = client.ip_address.to_string();
        let now = now_ms();
        let known_id = !client.client_id.is_empty();
        let config = self.store.update(|config| {
            let same_device = |device: &AllowedDevice| {
                (known_id && device.client_id == client.client_id)
                    || (device.client_id.is_empty() && device.last_ip == ip)
            };
            let previous = config
                .allowed_devices
                .iter()
                .find(|device| same_device(device));
            client.requested_at_ms = previous.map_or(now, |device| device.approved_at_ms);
            config.allowed_devices.retain(|device| !same_device(device));
            for device in config
                .allowed_devices
                .iter_mut()
                .filter(|device| device.last_ip == ip)
            {
                device.last_ip.clear();
            }
            config.allowed_devices.push(AllowedDevice {
                client_id: client.client_id.clone(),
                client_name: client.client_name.clone(),
                last_ip: ip,
                approved_at_ms: client.requested_at_ms,
                last_seen_ms: now,
            });
        })?;
        let association_changed = state.approved.iter().any(|(ip, previous)| {
            (*ip == client.ip_address && previous.client_id != client.client_id)
                || (known_id && *ip != client.ip_address && previous.client_id == client.client_id)
        });
        if association_changed {
            state.release_input();
        }
        state
            .approved
            .retain(|_, existing| !known_id || existing.client_id != client.client_id);
        state.pending.retain(|ip, pending| {
            *ip != client.ip_address && (!known_id || pending.client_id != client.client_id)
        });
        state.rejected_ips.remove(&client.ip_address);
        state.rejected_ids.remove(&client.client_id);
        state.approved.insert(client.ip_address, client);
        state.config = config;
        Ok(())
    }

    fn record_last_seen(&self, state: &mut State, ip: IpAddr, events: &mut Vec<ServerEvent>) {
        let Some(client) = state.approved.get(&ip) else {
            return;
        };
        let now = now_ms();
        let address = ip.to_string();
        let id = &client.client_id;
        let matches = |device: &AllowedDevice| device.client_id == *id && device.last_ip == address;
        if state
            .config
            .allowed_devices
            .iter()
            .any(|device| matches(device) && device.last_seen_ms != now)
        {
            match self.store.update(|config| {
                if let Some(device) = config
                    .allowed_devices
                    .iter_mut()
                    .find(|device| matches(device))
                {
                    device.last_seen_ms = now;
                }
            }) {
                Ok(config) => state.config = config,
                Err(error) => events.push(ServerEvent::Error(error.to_string())),
            }
        }
    }

    fn control(
        &self,
        state: &mut State,
        socket: &UdpSocket,
        from: SocketAddr,
        packet: &TouchpadPacket,
    ) {
        match *packet {
            TouchpadPacket::Move { dx, dy } => {
                state.mouse.move_cursor(dx, dy);
                if let Some(hit) = state.edge.evaluate_move(
                    self.backend.as_ref(),
                    dx,
                    dy,
                    !state.mouse.pressed_buttons().is_empty(),
                ) {
                    send(
                        socket,
                        from,
                        TouchpadPacket::EdgeHit {
                            edge: hit.edge,
                            ratio: hit.ratio,
                        },
                    );
                }
            }
            TouchpadPacket::Click { button } => state.mouse.click(button),
            TouchpadPacket::ButtonState { button, pressed } => {
                state.mouse.set_button_state(button, pressed);
                if pressed {
                    state.edge.reset();
                }
            }
            TouchpadPacket::Scroll { delta_y } => state.mouse.scroll(delta_y),
            TouchpadPacket::CursorEnter { edge, ratio } => {
                let (x, y) = EdgeDetector::compute_enter_position(
                    edge,
                    ratio,
                    &self.backend.monitor_bounds(),
                );
                state.mouse.warp(x, y);
                state.edge.on_cursor_enter();
            }
            TouchpadPacket::CursorLeave { .. } => state.release_input(),
            TouchpadPacket::EdgeConfig {
                left,
                right,
                top,
                bottom,
                overshoot_px,
            } => {
                state.edge.update_config(EdgeConfig {
                    left,
                    right,
                    top,
                    bottom,
                    overshoot_px,
                });
            }
            _ => {}
        }
    }
}

fn send(socket: &UdpSocket, to: SocketAddr, packet: TouchpadPacket) {
    if let Err(error) = socket.send_to(&packet.to_bytes(), to) {
        tracing::debug!(%to, %error, "UDP reply failed");
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
