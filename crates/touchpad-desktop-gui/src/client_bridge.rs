//! Background bridge managing Windows Service IPC vs local in-process server lifecycle.

use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;
#[cfg(windows)]
use std::time::Instant;

use touchpad_protocol::{HostOs, PrivilegeMode, TouchpadPacket};
use touchpad_server_core::{
    EventCallback, InputBackend, ServerEvent, ServerSnapshot, TouchpadServer,
};
use touchpad_storage::{default_user_desktop_config_path, ConfigStore, DesktopConfig};

use crate::telemetry::{describe_packet_pt_br, ActiveEdges};

#[cfg(windows)]
use touchpad_win_service::{ipc_client, ControlRequest};

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMode {
    Local,
    WindowsService,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxBackendDiagnostic {
    pub session_label: &'static str,
    pub injection_label: &'static str,
    pub hint: Option<String>,
    pub unavailable_error: Option<String>,
}

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone)]
pub struct BridgeState {
    pub mode: BridgeMode,
    pub snapshot: ServerSnapshot,
    pub host_os: HostOs,
    pub privilege_mode: PrivilegeMode,
    pub service_installed: bool,
    pub service_running: bool,
    pub service_transition_busy: bool,
    pub service_status_notice: Option<String>,
    pub linux_status: Option<LinuxBackendDiagnostic>,
    pub last_action_text: String,
    pub active_edges: ActiveEdges,
    pub last_error: Option<String>,
    pub config_revision: u64,
}

enum BridgeCommand {
    StartServer,
    StopServer,
    ApprovePairing(IpAddr),
    RejectPairing(IpAddr),
    RevokeDevice {
        client_id: String,
        ip_address: String,
    },
    UpdateConfig(DesktopConfig),
    #[cfg(windows)]
    InstallWindowsService,
    #[cfg(windows)]
    UninstallWindowsService,
    #[cfg(windows)]
    PrepareRelaunchAsAdmin(Sender<Result<(), String>>),
    Shutdown,
}

/// Returns whether the initial window should start hidden in the tray (Windows only).
pub fn initial_start_minimized_to_tray() -> bool {
    #[cfg(windows)]
    {
        let path = if ipc_client::is_service_running() {
            let service_cfg = touchpad_storage::windows_service_config_path();
            if service_cfg.exists() {
                service_cfg
            } else {
                default_user_desktop_config_path()
            }
        } else {
            default_user_desktop_config_path()
        };
        ConfigStore::<DesktopConfig>::new(path)
            .map(|store| store.get().start_minimized_to_tray)
            .unwrap_or(false)
    }

    #[cfg(not(windows))]
    {
        false
    }
}

/// Manages a local in-process [`TouchpadServer`] and [`ConfigStore<DesktopConfig>`].
pub struct LocalServerRuntime {
    backend: Arc<dyn InputBackend>,
    store: Arc<ConfigStore<DesktopConfig>>,
    server: Option<TouchpadServer>,
    event_callback: Option<EventCallback>,
    user_stopped: bool,
}

impl LocalServerRuntime {
    pub fn new(
        backend: Arc<dyn InputBackend>,
        store: Arc<ConfigStore<DesktopConfig>>,
        event_callback: Option<EventCallback>,
        bind_port_override: Option<u16>,
    ) -> Self {
        let server = Self::build_server(&backend, &store, &event_callback, bind_port_override);
        Self {
            backend,
            store,
            server: Some(server),
            event_callback,
            user_stopped: false,
        }
    }

    fn build_server(
        backend: &Arc<dyn InputBackend>,
        store: &Arc<ConfigStore<DesktopConfig>>,
        event_callback: &Option<EventCallback>,
        port_override: Option<u16>,
    ) -> TouchpadServer {
        let mut server = match port_override {
            Some(port) => TouchpadServer::with_port(Arc::clone(backend), Arc::clone(store), port),
            None => TouchpadServer::new(Arc::clone(backend), Arc::clone(store)),
        };
        if let Some(cb) = event_callback {
            server = server.with_event_callback(Arc::clone(cb));
        }
        server
    }

    pub fn start(&mut self) -> Result<(), String> {
        let Some(server) = self.server.as_mut() else {
            return Err("Backend de entrada indisponível neste ambiente.".to_string());
        };
        self.user_stopped = false;
        server.start().map_err(|err| err.to_string())
    }

    pub fn stop(&mut self) {
        if let Some(server) = self.server.as_mut() {
            server.stop();
        }
    }

    pub fn mark_user_stopped(&mut self) {
        self.user_stopped = true;
        self.stop();
    }

    #[cfg(windows)]
    pub fn is_user_stopped(&self) -> bool {
        self.user_stopped
    }

    #[cfg(any(windows, test))]
    pub fn is_running(&self) -> bool {
        self.server.as_ref().is_some_and(TouchpadServer::is_running)
    }

    pub fn snapshot(&self) -> ServerSnapshot {
        if let Some(server) = &self.server {
            server.snapshot()
        } else {
            let config = self.store.get();
            ServerSnapshot {
                running: false,
                actual_port: config.port,
                packet_count: 0,
                last_client_ip: None,
                pending_pairings: Vec::new(),
                approved_clients: Vec::new(),
                config,
            }
        }
    }

    pub fn approve_pairing(&self, ip: IpAddr) -> Result<(), String> {
        let Some(server) = &self.server else {
            return Err("Servidor local indisponível.".to_string());
        };
        server.approve_pairing(ip).map_err(|err| err.to_string())
    }

    pub fn reject_pairing(&self, ip: IpAddr) -> Result<(), String> {
        let Some(server) = &self.server else {
            return Err("Servidor local indisponível.".to_string());
        };
        server.reject_pairing(ip).map_err(|err| err.to_string())
    }

    pub fn revoke_device(&self, client_id: &str, ip_address: &str) -> Result<(), String> {
        let client_id = client_id.trim();
        let ip_address = ip_address.trim();
        if client_id.is_empty() && ip_address.is_empty() {
            return Err("Identificador e IP do dispositivo vazios.".to_string());
        }

        if let Some(server) = &self.server {
            if !client_id.is_empty() {
                server
                    .revoke_device(client_id)
                    .map_err(|err| err.to_string())?;
            }
            if !ip_address.is_empty() && ip_address != client_id {
                server
                    .revoke_device(ip_address)
                    .map_err(|err| err.to_string())?;
            }
        } else {
            self.store
                .update(|cfg| {
                    cfg.allowed_devices.retain(|device| {
                        (client_id.is_empty() || device.client_id != client_id)
                            && (ip_address.is_empty() || device.last_ip != ip_address)
                    });
                })
                .map_err(|err| err.to_string())?;
        }

        Ok(())
    }

    pub fn update_config(&mut self, mut incoming: DesktopConfig) -> Result<DesktopConfig, String> {
        incoming.sanitize();
        let old_port = self.store.get().port;

        let updated = self
            .store
            .update(|cfg| {
                cfg.computer_name = incoming.computer_name.clone();
                cfg.port = incoming.port;
                cfg.sensitivity = incoming.sensitivity;
                cfg.scroll_sensitivity = incoming.scroll_sensitivity;
                cfg.require_pairing_approval = incoming.require_pairing_approval;
                cfg.edge_overshoot_px = incoming.edge_overshoot_px;
                cfg.edge_cooldown_ms = incoming.edge_cooldown_ms;
                cfg.start_minimized_to_tray = incoming.start_minimized_to_tray;
            })
            .map_err(|err| err.to_string())?;

        if let Some(server) = self.server.as_mut() {
            server
                .update_settings(
                    updated.sensitivity,
                    updated.scroll_sensitivity,
                    updated.edge_cooldown_ms,
                    updated.require_pairing_approval,
                )
                .map_err(|err| err.to_string())?;

            if updated.port != old_port {
                let was_running = server.is_running();
                server.stop();
                let mut next =
                    Self::build_server(&self.backend, &self.store, &self.event_callback, None);
                if was_running && !self.user_stopped {
                    next.start().map_err(|err| err.to_string())?;
                }
                *server = next;
            }
        }

        Ok(updated)
    }

    pub fn flush_config(&self) {
        let cfg = self.store.get();
        let _ = self.store.save(cfg);
    }

    #[cfg(windows)]
    pub fn reload_store_from_disk(&mut self) -> Result<(), String> {
        let path = self.store.path().to_path_buf();
        let fresh = ConfigStore::<DesktopConfig>::new(path).map_err(|err| err.to_string())?;
        self.store = Arc::new(fresh);
        let was_running = self.is_running();
        self.stop();
        if self.server.is_some() {
            let mut next =
                Self::build_server(&self.backend, &self.store, &self.event_callback, None);
            if was_running && !self.user_stopped {
                next.start().map_err(|err| err.to_string())?;
            }
            self.server = Some(next);
        }
        Ok(())
    }
}

pub struct ClientBridge {
    shared: Arc<Mutex<BridgeState>>,
    tx: Sender<BridgeCommand>,
    shutdown_called: AtomicBool,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl ClientBridge {
    pub fn start(egui_ctx: egui::Context) -> Self {
        let config_path = default_user_desktop_config_path();
        let store = Arc::new(
            ConfigStore::<DesktopConfig>::new(config_path)
                .unwrap_or_else(|_| {
                    ConfigStore::new(std::env::temp_dir().join("touchpad-remote-fallback.json"))
                        .expect("fallback config store")
                }),
        );
        let initial_config = store.get();

        let (backend_opt, host_os, privilege_mode, linux_status) = init_platform_backend();

        let initial_snapshot = ServerSnapshot {
            running: false,
            actual_port: initial_config.port,
            packet_count: 0,
            last_client_ip: None,
            pending_pairings: Vec::new(),
            approved_clients: Vec::new(),
            config: initial_config,
        };

        let shared = Arc::new(Mutex::new(BridgeState {
            mode: BridgeMode::Local,
            snapshot: initial_snapshot,
            host_os,
            privilege_mode,
            service_installed: false,
            service_running: false,
            service_transition_busy: false,
            service_status_notice: None,
            linux_status,
            last_action_text: "Nenhuma ação recebida ainda".to_string(),
            active_edges: ActiveEdges::default(),
            last_error: None,
            config_revision: 1,
        }));

        let (tx, rx) = mpsc::channel::<BridgeCommand>();
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("touchpad-gui-bridge".to_string())
            .spawn(move || {
                bridge_worker_loop(worker_shared, store, backend_opt, egui_ctx, rx);
            })
            .expect("spawn touchpad-gui-bridge thread");

        Self {
            shared,
            tx,
            shutdown_called: AtomicBool::new(false),
            worker: Mutex::new(Some(worker)),
        }
    }

    pub fn state(&self) -> BridgeState {
        self.shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn start_server(&self) {
        let _ = self.tx.send(BridgeCommand::StartServer);
    }

    pub fn stop_server(&self) {
        let _ = self.tx.send(BridgeCommand::StopServer);
    }

    pub fn approve_pairing(&self, ip: IpAddr) {
        let _ = self.tx.send(BridgeCommand::ApprovePairing(ip));
    }

    pub fn reject_pairing(&self, ip: IpAddr) {
        let _ = self.tx.send(BridgeCommand::RejectPairing(ip));
    }

    pub fn revoke_device(&self, client_id: String, ip_address: String) {
        let _ = self.tx.send(BridgeCommand::RevokeDevice {
            client_id,
            ip_address,
        });
    }

    pub fn update_config(&self, config: DesktopConfig) {
        let _ = self.tx.send(BridgeCommand::UpdateConfig(config));
    }

    #[cfg(windows)]
    pub fn install_windows_service(&self) {
        let _ = self.tx.send(BridgeCommand::InstallWindowsService);
    }

    #[cfg(windows)]
    pub fn uninstall_windows_service(&self) {
        let _ = self.tx.send(BridgeCommand::UninstallWindowsService);
    }

    #[cfg(windows)]
    pub fn prepare_relaunch_as_admin(&self) -> Result<(), String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        self.tx
            .send(BridgeCommand::PrepareRelaunchAsAdmin(reply_tx))
            .map_err(|_| "Bridge encerrada.".to_string())?;
        reply_rx
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| "Tempo esgotado ao parar servidor local.".to_string())?
    }

    pub fn shutdown(&self) {
        if self.shutdown_called.swap(true, Ordering::AcqRel) {
            return;
        }
        let _ = self.tx.send(BridgeCommand::Shutdown);
        if let Some(handle) = self
            .worker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            let _ = handle.join();
        }
    }
}

impl Drop for ClientBridge {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn init_platform_backend() -> (
    Option<Arc<dyn InputBackend>>,
    HostOs,
    PrivilegeMode,
    Option<LinuxBackendDiagnostic>,
) {
    #[cfg(windows)]
    {
        let backend: Arc<dyn InputBackend> =
            Arc::new(touchpad_input_win::WindowsInputBackend::new());
        let privilege = backend.privilege_mode();
        (Some(backend), HostOs::Windows, privilege, None)
    }

    #[cfg(target_os = "linux")]
    {
        match touchpad_input_linux::LinuxInputBackend::new() {
            Ok(backend) => {
                let status = *backend.status();
                let privilege = backend.privilege_mode();
                let diag = LinuxBackendDiagnostic {
                    session_label: session_label(status.session),
                    injection_label: injection_label(status.injection),
                    hint: status.hint.map(str::to_string),
                    unavailable_error: None,
                };
                (Some(Arc::new(backend)), HostOs::Linux, privilege, Some(diag))
            }
            Err(err) => {
                let diag = LinuxBackendDiagnostic {
                    session_label: session_label(err.status.session),
                    injection_label: injection_label(err.status.injection),
                    hint: err.status.hint.map(str::to_string),
                    unavailable_error: Some(err.to_string()),
                };
                (None, HostOs::Linux, PrivilegeMode::User, Some(diag))
            }
        }
    }

    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let backend: Arc<dyn InputBackend> =
            Arc::new(touchpad_server_core::DryRunBackend::default());
        let os = backend.os_kind();
        let privilege = backend.privilege_mode();
        (Some(backend), os, privilege, None)
    }
}

#[cfg(target_os = "linux")]
fn session_label(kind: touchpad_input_linux::SessionKind) -> &'static str {
    match kind {
        touchpad_input_linux::SessionKind::Wayland => "Wayland",
        touchpad_input_linux::SessionKind::X11 => "X11",
        touchpad_input_linux::SessionKind::Headless => "Headless",
    }
}

#[cfg(target_os = "linux")]
fn injection_label(method: touchpad_input_linux::InjectionMethod) -> &'static str {
    match method {
        touchpad_input_linux::InjectionMethod::Uinput => "uinput",
        touchpad_input_linux::InjectionMethod::Xtest => "XTest",
        touchpad_input_linux::InjectionMethod::Unavailable => "Indisponível",
    }
}

struct WorkerState {
    shared: Arc<Mutex<BridgeState>>,
    egui_ctx: egui::Context,
    runtime: Option<LocalServerRuntime>,
    service_mode: bool,
    #[cfg(windows)]
    install_grace_until: Option<Instant>,
    #[cfg(windows)]
    uninstall_grace_until: Option<Instant>,
}

impl WorkerState {
    fn sync_from_local_snapshot(&mut self) {
        let Some(runtime) = &self.runtime else {
            return;
        };
        let snapshot = runtime.snapshot();
        let mut guard = self
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let config_changed = guard.snapshot.config != snapshot.config;
        guard.snapshot = snapshot;
        if config_changed {
            guard.config_revision = guard.config_revision.wrapping_add(1);
        }
        drop(guard);
        self.egui_ctx.request_repaint();
    }

    fn set_error(&self, err: Option<String>) {
        let mut guard = self
            .shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.last_error = err;
        drop(guard);
        self.egui_ctx.request_repaint();
    }

    #[cfg(windows)]
    fn refresh_windows_service(&mut self) {
        let installed = ipc_client::is_service_installed();
        let running = ipc_client::is_service_running();

        if running && !self.service_mode {
            if let Some(runtime) = self.runtime.as_mut() {
                runtime.stop();
            }
            self.service_mode = true;
            self.install_grace_until = None;
            let mut guard = self
                .shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.mode = BridgeMode::WindowsService;
            guard.privilege_mode = PrivilegeMode::System;
            guard.service_installed = installed;
            guard.service_running = true;
            guard.service_transition_busy = false;
            guard.service_status_notice = Some("Conectado ao Serviço UAC (LocalSystem).".to_string());
            guard.last_error = None;
        } else if !running && self.service_mode {
            self.service_mode = false;
            self.uninstall_grace_until = None;
            if let Some(runtime) = self.runtime.as_mut() {
                let _ = runtime.reload_store_from_disk();
                let in_install_grace = self
                    .install_grace_until
                    .is_some_and(|until| Instant::now() < until);
                if !in_install_grace && !runtime.is_user_stopped() {
                    if let Err(err) = runtime.start() {
                        self.set_error(Some(format!("Erro ao retomar servidor local: {err}")));
                    }
                }
            }
            let privilege = self
                .runtime
                .as_ref()
                .map(|r| r.backend.privilege_mode())
                .unwrap_or(PrivilegeMode::User);
            let mut guard = self
                .shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.mode = BridgeMode::Local;
            guard.privilege_mode = privilege;
            guard.service_installed = installed;
            guard.service_running = false;
            guard.service_transition_busy = false;
            guard.service_status_notice = if installed {
                Some("Serviço UAC parado — operando em modo portátil.".to_string())
            } else {
                None
            };
        } else {
            let now = Instant::now();
            let mut expired_install = false;
            if let Some(until) = self.install_grace_until {
                if now >= until {
                    self.install_grace_until = None;
                    expired_install = true;
                }
            }
            if let Some(until) = self.uninstall_grace_until {
                if now >= until || !installed {
                    self.uninstall_grace_until = None;
                    let mut guard = self
                        .shared
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    guard.service_transition_busy = false;
                    if !installed {
                        guard.service_status_notice =
                            Some("Serviço UAC removido. Modo portátil ativo.".to_string());
                    }
                }
            }
            if expired_install && !running {
                if let Some(runtime) = self.runtime.as_mut() {
                    if !runtime.is_user_stopped() {
                        let _ = runtime.start();
                    }
                }
                let mut guard = self
                    .shared
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard.service_transition_busy = false;
                guard.service_status_notice = Some(
                    "O Serviço UAC não iniciou a tempo. Modo portátil reativado.".to_string(),
                );
            }
            let mut guard = self
                .shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.service_installed = installed;
            guard.service_running = running;
        }

        if self.service_mode {
            match ipc_client::query_control(&ControlRequest::GetSnapshot)
                .and_then(|resp| resp.into_snapshot())
            {
                Ok(snapshot) => {
                    let mut guard = self
                        .shared
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    let prev_packets = guard.snapshot.packet_count;
                    let config_changed = guard.snapshot.config != snapshot.config;
                    if snapshot.packet_count > prev_packets {
                        guard.last_action_text =
                            "Atividade recebida pelo Serviço UAC".to_string();
                    }
                    guard.snapshot = snapshot;
                    guard.last_error = None;
                    if config_changed {
                        guard.config_revision = guard.config_revision.wrapping_add(1);
                    }
                }
                Err(err) => {
                    let mut guard = self
                        .shared
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    guard.last_error = Some(format!("Falha de comunicação com Serviço UAC: {err}"));
                }
            }
            self.egui_ctx.request_repaint();
        } else {
            self.sync_from_local_snapshot();
        }
    }

    fn execute_command(&mut self, cmd: BridgeCommand) -> bool {
        match cmd {
            BridgeCommand::Shutdown => {
                if let Some(runtime) = self.runtime.as_mut() {
                    runtime.stop();
                    runtime.flush_config();
                }
                return false;
            }
            BridgeCommand::StartServer => {
                if self.service_mode {
                    #[cfg(windows)]
                    self.send_service_control(ControlRequest::StartServer);
                } else if let Some(runtime) = self.runtime.as_mut() {
                    match runtime.start() {
                        Ok(()) => self.set_error(None),
                        Err(err) => self.set_error(Some(format!(
                            "Não foi possível abrir a porta UDP: {err}"
                        ))),
                    }
                    self.sync_from_local_snapshot();
                }
            }
            BridgeCommand::StopServer => {
                if self.service_mode {
                    #[cfg(windows)]
                    self.send_service_control(ControlRequest::StopServer);
                } else if let Some(runtime) = self.runtime.as_mut() {
                    runtime.mark_user_stopped();
                    self.sync_from_local_snapshot();
                }
            }
            BridgeCommand::ApprovePairing(ip) => {
                if self.service_mode {
                    #[cfg(windows)]
                    self.send_service_control(ControlRequest::ApprovePairing {
                        ip_or_id: ip.to_string(),
                    });
                } else if let Some(runtime) = self.runtime.as_ref() {
                    match runtime.approve_pairing(ip) {
                        Ok(()) => self.set_error(None),
                        Err(err) => self.set_error(Some(err)),
                    }
                    self.sync_from_local_snapshot();
                }
            }
            BridgeCommand::RejectPairing(ip) => {
                if self.service_mode {
                    #[cfg(windows)]
                    self.send_service_control(ControlRequest::RejectPairing {
                        ip_or_id: ip.to_string(),
                    });
                } else if let Some(runtime) = self.runtime.as_ref() {
                    match runtime.reject_pairing(ip) {
                        Ok(()) => self.set_error(None),
                        Err(err) => self.set_error(Some(err)),
                    }
                    self.sync_from_local_snapshot();
                }
            }
            BridgeCommand::RevokeDevice {
                client_id,
                ip_address,
            } => {
                if self.service_mode {
                    #[cfg(windows)]
                    self.send_service_control(ControlRequest::RevokeDevice {
                        client_id,
                        ip_address,
                    });
                } else if let Some(runtime) = self.runtime.as_ref() {
                    match runtime.revoke_device(&client_id, &ip_address) {
                        Ok(()) => self.set_error(None),
                        Err(err) => self.set_error(Some(err)),
                    }
                    self.sync_from_local_snapshot();
                }
            }
            BridgeCommand::UpdateConfig(config) => {
                if self.service_mode {
                    #[cfg(windows)]
                    self.send_service_control(ControlRequest::UpdateConfig(config));
                } else if let Some(runtime) = self.runtime.as_mut() {
                    match runtime.update_config(config) {
                        Ok(_) => self.set_error(None),
                        Err(err) => self.set_error(Some(err)),
                    }
                    self.sync_from_local_snapshot();
                }
            }
            #[cfg(windows)]
            BridgeCommand::InstallWindowsService => {
                self.handle_install_service();
            }
            #[cfg(windows)]
            BridgeCommand::UninstallWindowsService => {
                self.handle_uninstall_service();
            }
            #[cfg(windows)]
            BridgeCommand::PrepareRelaunchAsAdmin(reply_tx) => {
                if let Some(runtime) = self.runtime.as_mut() {
                    runtime.stop();
                    runtime.flush_config();
                }
                self.sync_from_local_snapshot();
                let _ = reply_tx.send(Ok(()));
            }
        }
        true
    }

    #[cfg(windows)]
    fn send_service_control(&mut self, req: ControlRequest) {
        match ipc_client::query_control(&req).and_then(|resp| resp.into_result()) {
            Ok(()) => {
                self.set_error(None);
                self.refresh_windows_service();
            }
            Err(err) => {
                self.set_error(Some(format!("Erro no Serviço UAC: {err}")));
            }
        }
    }

    #[cfg(windows)]
    fn handle_install_service(&mut self) {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|current| crate::telemetry::resolve_service_exe_path(&current));
        let Some(service_exe) = exe else {
            self.set_error(Some(
                "Não foi encontrado touchpad-win-service.exe na mesma pasta do TouchPadRemote.exe."
                    .to_string(),
            ));
            return;
        };

        {
            let mut guard = self
                .shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.service_transition_busy = true;
            guard.service_status_notice =
                Some("Liberando porta UDP e solicitando permissão UAC...".to_string());
            guard.last_error = None;
        }
        self.egui_ctx.request_repaint();

        if let Some(runtime) = self.runtime.as_mut() {
            runtime.stop();
            runtime.flush_config();
        }
        self.sync_from_local_snapshot();

        match ipc_client::install_service_elevated(&service_exe) {
            Ok(()) => {
                self.install_grace_until = Some(Instant::now() + Duration::from_secs(20));
                let mut guard = self
                    .shared
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard.service_status_notice =
                    Some("Aguardando o Serviço UAC iniciar...".to_string());
            }
            Err(err) => {
                self.install_grace_until = None;
                if let Some(runtime) = self.runtime.as_mut() {
                    if !runtime.is_user_stopped() {
                        let _ = runtime.start();
                    }
                }
                let mut guard = self
                    .shared
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard.service_transition_busy = false;
                guard.service_status_notice = None;
                guard.last_error = Some(format!(
                    "Instalação do Serviço UAC cancelada ou falhou: {err}"
                ));
                drop(guard);
                self.sync_from_local_snapshot();
            }
        }
        self.egui_ctx.request_repaint();
    }

    #[cfg(windows)]
    fn handle_uninstall_service(&mut self) {
        let exe = std::env::current_exe()
            .ok()
            .and_then(|current| crate::telemetry::resolve_service_exe_path(&current));
        let Some(service_exe) = exe else {
            self.set_error(Some(
                "Não foi encontrado touchpad-win-service.exe na mesma pasta do TouchPadRemote.exe."
                    .to_string(),
            ));
            return;
        };

        {
            let mut guard = self
                .shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            guard.service_transition_busy = true;
            guard.service_status_notice =
                Some("Solicitando permissão UAC para remover o serviço...".to_string());
            guard.last_error = None;
        }
        self.egui_ctx.request_repaint();

        match ipc_client::uninstall_service_elevated(&service_exe) {
            Ok(()) => {
                self.uninstall_grace_until = Some(Instant::now() + Duration::from_secs(15));
                let mut guard = self
                    .shared
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard.service_status_notice =
                    Some("Aguardando a desinstalação do Serviço UAC...".to_string());
            }
            Err(err) => {
                self.uninstall_grace_until = None;
                let mut guard = self
                    .shared
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard.service_transition_busy = false;
                guard.service_status_notice = None;
                guard.last_error = Some(format!(
                    "Remoção do Serviço UAC cancelada ou falhou: {err}"
                ));
            }
        }
        self.egui_ctx.request_repaint();
    }
}

fn bridge_worker_loop(
    shared: Arc<Mutex<BridgeState>>,
    store: Arc<ConfigStore<DesktopConfig>>,
    backend_opt: Option<Arc<dyn InputBackend>>,
    egui_ctx: egui::Context,
    rx: Receiver<BridgeCommand>,
) {
    let event_shared = Arc::clone(&shared);
    let event_ctx = egui_ctx.clone();

    let event_cb: EventCallback = Arc::new(move |event| {
        {
            let mut guard = event_shared
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match event {
                ServerEvent::StateChanged { running, port } => {
                    guard.snapshot.running = running;
                    if port > 0 {
                        guard.snapshot.actual_port = port;
                    }
                }
                ServerEvent::PacketReceived {
                    packet,
                    client_ip,
                    total_packets,
                } => {
                    guard.snapshot.last_client_ip = Some(client_ip);
                    guard.snapshot.packet_count = total_packets;
                    guard.last_action_text = describe_packet_pt_br(&packet);
                    if let TouchpadPacket::EdgeConfig {
                        left,
                        right,
                        top,
                        bottom,
                        ..
                    } = packet
                    {
                        guard.active_edges = ActiveEdges {
                            left,
                            right,
                            top,
                            bottom,
                        };
                    }
                }
                ServerEvent::DiscoveryRequested {
                    client_name,
                    client_ip,
                    ..
                } => {
                    guard.snapshot.last_client_ip = Some(client_ip);
                    guard.last_action_text = format!("Descoberta de {client_name}");
                }
                ServerEvent::PingReceived { client_ip } => {
                    guard.snapshot.last_client_ip = Some(client_ip);
                    guard.last_action_text = "Ping respondido".to_string();
                }
                ServerEvent::PairingRequested(client) => {
                    guard.snapshot.last_client_ip = Some(client.ip_address);
                    guard.last_action_text =
                        format!("Pedido de pareamento: {}", client.client_name);
                }
                ServerEvent::PairingStateChanged => {}
                ServerEvent::Error(msg) => {
                    guard.last_error = Some(msg);
                }
            }
        }
        event_ctx.request_repaint();
    });

    #[cfg(windows)]
    let service_already_running = ipc_client::is_service_running();
    #[cfg(not(windows))]
    let service_already_running = false;

    let runtime = backend_opt.map(|backend| {
        let mut runtime = LocalServerRuntime::new(backend, store, Some(event_cb), None);
        if !service_already_running {
            if let Err(err) = runtime.start() {
                let mut guard = shared
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard.last_error = Some(format!(
                    "Não foi possível abrir a porta UDP {}: {err}",
                    guard.snapshot.config.port
                ));
            }
        }
        runtime
    });

    let mut worker = WorkerState {
        shared,
        egui_ctx,
        runtime,
        service_mode: false,
        #[cfg(windows)]
        install_grace_until: None,
        #[cfg(windows)]
        uninstall_grace_until: None,
    };

    #[cfg(windows)]
    worker.refresh_windows_service();
    #[cfg(not(windows))]
    worker.sync_from_local_snapshot();

    loop {
        match rx.recv_timeout(Duration::from_millis(350)) {
            Ok(cmd) => {
                if !worker.execute_command(cmd) {
                    break;
                }
                while let Ok(next_cmd) = rx.try_recv() {
                    if !worker.execute_command(next_cmd) {
                        return;
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }

        #[cfg(windows)]
        worker.refresh_windows_service();

        #[cfg(not(windows))]
        worker.sync_from_local_snapshot();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use touchpad_server_core::{DryRunBackend, DryRunConfig};

    #[test]
    fn local_server_runtime_persists_settings_and_paired_devices() {
        let temp_dir = std::env::temp_dir().join(format!(
            "touchpad-gui-bridge-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();
        let cfg_path = temp_dir.join("config.json");

        let store = Arc::new(ConfigStore::<DesktopConfig>::new(cfg_path.clone()).unwrap());
        let backend: Arc<dyn InputBackend> = Arc::new(DryRunBackend::new(DryRunConfig::default()));

        let mut runtime = LocalServerRuntime::new(backend, Arc::clone(&store), None, Some(0));
        runtime.start().unwrap();
        assert!(runtime.is_running());

        let mut next = runtime.snapshot().config;
        next.sensitivity = 2.4;
        next.scroll_sensitivity = 1.7;
        next.edge_overshoot_px = 64;
        next.start_minimized_to_tray = true;
        next.require_pairing_approval = false;
        let updated = runtime.update_config(next).unwrap();
        assert!((updated.sensitivity - 2.4).abs() < 0.01);
        assert_eq!(updated.edge_overshoot_px, 64);
        assert!(updated.start_minimized_to_tray);
        assert!(!updated.require_pairing_approval);

        let ip: IpAddr = "192.168.1.77".parse().unwrap();
        runtime.approve_pairing(ip).unwrap();
        let snap = runtime.snapshot();
        assert_eq!(snap.approved_clients.len(), 1);
        assert_eq!(snap.config.allowed_devices.len(), 1);

        runtime.revoke_device("", "192.168.1.77").unwrap();
        let snap_after = runtime.snapshot();
        assert!(snap_after.approved_clients.is_empty());
        assert!(snap_after.config.allowed_devices.is_empty());

        runtime.stop();
        assert!(!runtime.is_running());
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
