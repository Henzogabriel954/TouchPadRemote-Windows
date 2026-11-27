//! Control RPC dispatcher (`ManagedServer`) and Windows Named Pipe acceptor (`ControlPipeServer`).

use std::net::IpAddr;
use std::sync::Arc;

use touchpad_server_core::{InputBackend, ServerSnapshot, TouchpadServer};
use touchpad_storage::{ConfigStore, DesktopConfig};

use crate::protocol::{ControlRequest, ControlResponse};

/// Owns the running [`TouchpadServer`] and persisted [`ConfigStore<DesktopConfig>`],
/// handling [`ControlRequest`] commands safely.
pub struct ManagedServer {
    server: TouchpadServer,
    store: Arc<ConfigStore<DesktopConfig>>,
    backend: Arc<dyn InputBackend>,
    bind_port_override: Option<u16>,
}

impl ManagedServer {
    /// Creates a [`ManagedServer`] using the port configured in `store`.
    pub fn new(backend: Arc<dyn InputBackend>, store: Arc<ConfigStore<DesktopConfig>>) -> Self {
        let server = TouchpadServer::new(Arc::clone(&backend), Arc::clone(&store));
        Self {
            server,
            store,
            backend,
            bind_port_override: None,
        }
    }

    /// Creates a [`ManagedServer`] binding to `port_override` (for instance, `0` in tests).
    pub fn with_port_override(
        backend: Arc<dyn InputBackend>,
        store: Arc<ConfigStore<DesktopConfig>>,
        port_override: u16,
    ) -> Self {
        let server =
            TouchpadServer::with_port(Arc::clone(&backend), Arc::clone(&store), port_override);
        Self {
            server,
            store,
            backend,
            bind_port_override: Some(port_override),
        }
    }

    /// Starts the underlying UDP server.
    pub fn start(&mut self) -> Result<(), String> {
        self.server.start().map_err(|err| err.to_string())
    }

    /// Stops the underlying UDP server.
    pub fn stop(&mut self) {
        self.server.stop();
    }

    /// Returns the current server snapshot.
    pub fn snapshot(&self) -> ServerSnapshot {
        self.server.snapshot()
    }

    /// Executes a single [`ControlRequest`] against the managed server and config store.
    pub fn handle_request(&mut self, req: ControlRequest) -> ControlResponse {
        match req {
            ControlRequest::GetSnapshot => ControlResponse::Snapshot(self.server.snapshot()),
            ControlRequest::ApprovePairing { ip_or_id } => {
                match self.approve_pairing(&ip_or_id) {
                    Ok(()) => ControlResponse::Ok,
                    Err(err) => ControlResponse::Error(err),
                }
            }
            ControlRequest::RejectPairing { ip_or_id } => {
                match self.reject_pairing(&ip_or_id) {
                    Ok(()) => ControlResponse::Ok,
                    Err(err) => ControlResponse::Error(err),
                }
            }
            ControlRequest::RevokeDevice {
                client_id,
                ip_address,
            } => match self.revoke_device(&client_id, &ip_address) {
                Ok(()) => ControlResponse::Ok,
                Err(err) => ControlResponse::Error(err),
            },
            ControlRequest::UpdateConfig(config) => match self.update_config(config) {
                Ok(()) => ControlResponse::Ok,
                Err(err) => ControlResponse::Error(err),
            },
            ControlRequest::StartServer => match self.server.start() {
                Ok(()) => ControlResponse::Ok,
                Err(err) => ControlResponse::Error(err.to_string()),
            },
            ControlRequest::StopServer => {
                self.server.stop();
                ControlResponse::Ok
            }
        }
    }

    fn approve_pairing(&self, ip_or_id: &str) -> Result<(), String> {
        let target = ip_or_id.trim();
        if target.is_empty() {
            return Err("Empty pairing target".to_string());
        }

        let ip = if let Ok(ip) = target.parse::<IpAddr>() {
            ip
        } else {
            let snapshot = self.server.snapshot();
            let from_live = snapshot
                .pending_pairings
                .iter()
                .chain(snapshot.approved_clients.iter())
                .find(|client| client.client_id == target)
                .map(|client| client.ip_address);

            let from_store = from_live.or_else(|| {
                self.store
                    .get()
                    .allowed_devices
                    .iter()
                    .find(|device| device.client_id == target)
                    .and_then(|device| device.last_ip.parse::<IpAddr>().ok())
            });

            from_store.ok_or_else(|| format!("No pending or known client matching '{target}'"))?
        };

        self.server
            .approve_pairing(ip)
            .map_err(|err| err.to_string())
    }

    fn reject_pairing(&self, ip_or_id: &str) -> Result<(), String> {
        let target = ip_or_id.trim();
        if target.is_empty() {
            return Err("Empty pairing target".to_string());
        }

        if let Ok(ip) = target.parse::<IpAddr>() {
            self.server
                .reject_pairing(ip)
                .map_err(|err| err.to_string())
        } else {
            self.server
                .revoke_device(target)
                .map_err(|err| err.to_string())
        }
    }

    fn revoke_device(&self, client_id: &str, ip_address: &str) -> Result<(), String> {
        let client_id = client_id.trim();
        let ip_address = ip_address.trim();
        if client_id.is_empty() && ip_address.is_empty() {
            return Err("Both client_id and ip_address are empty".to_string());
        }

        if !client_id.is_empty() {
            self.server
                .revoke_device(client_id)
                .map_err(|err| err.to_string())?;
        }
        if !ip_address.is_empty() && ip_address != client_id {
            self.server
                .revoke_device(ip_address)
                .map_err(|err| err.to_string())?;
        }
        Ok(())
    }

    fn update_config(&mut self, mut incoming: DesktopConfig) -> Result<(), String> {
        incoming.sanitize();
        let old_port = self.store.get().port;

        // Update all 8 persisted settings fields while preserving paired allowed_devices.
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

        // Re-applies full DesktopConfig from `store` into live TouchpadServer state.
        self.server
            .update_settings(
                updated.sensitivity,
                updated.scroll_sensitivity,
                updated.edge_cooldown_ms,
                updated.require_pairing_approval,
            )
            .map_err(|err| err.to_string())?;

        if updated.port != old_port && self.bind_port_override.is_none() {
            let was_running = self.server.is_running();
            self.server.stop();
            let mut next = TouchpadServer::new(Arc::clone(&self.backend), Arc::clone(&self.store));
            if was_running {
                next.start().map_err(|err| err.to_string())?;
            }
            self.server = next;
        }

        Ok(())
    }
}

#[cfg(windows)]
mod win_server {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use crate::pipe::{connect_named_pipe_with_stop, create_named_pipe_server, NamedPipeStream};
    use crate::protocol::{ControlRequest, ControlResponse};

    use super::ManagedServer;

    const MAX_CONTROL_PIPE_INSTANCES: u32 = 16;
    const MAX_CONCURRENT_CLIENTS: usize = 8;
    const CONTROL_READ_TIMEOUT_MS: u32 = 2_000;
    const CONTROL_WRITE_TIMEOUT_MS: u32 = 2_000;

    /// Named Pipe RPC server serving [`ControlRequest`] calls over `\\.\pipe\TouchPadRemoteControl`.
    pub struct ControlPipeServer {
        stop_flag: Arc<AtomicBool>,
        active_workers: Arc<AtomicUsize>,
        worker: Option<JoinHandle<()>>,
    }

    impl ControlPipeServer {
        /// Synchronously creates the first pipe instance (`FILE_FLAG_FIRST_PIPE_INSTANCE`)
        /// and launches the acceptor thread.
        pub fn start(
            pipe_name: String,
            sddl: Option<String>,
            managed: Arc<Mutex<ManagedServer>>,
            stop_flag: Arc<AtomicBool>,
        ) -> Result<Self, String> {
            let initial_pipe = create_named_pipe_server(
                &pipe_name,
                sddl.as_deref(),
                MAX_CONTROL_PIPE_INSTANCES,
                true,
            )?;
            let active_workers = Arc::new(AtomicUsize::new(0));

            let thread_stop = Arc::clone(&stop_flag);
            let thread_workers = Arc::clone(&active_workers);
            let worker = thread::Builder::new()
                .name("touchpad-control-pipe".to_string())
                .spawn(move || {
                    let mut next_pipe = Some(initial_pipe);
                    while !thread_stop.load(Ordering::Acquire) {
                        let listener = match next_pipe.take() {
                            Some(pipe) => pipe,
                            None => match create_named_pipe_server(
                                &pipe_name,
                                sddl.as_deref(),
                                MAX_CONTROL_PIPE_INSTANCES,
                                false,
                            ) {
                                Ok(pipe) => pipe,
                                Err(err) => {
                                    tracing::warn!(%err, "Retrying control pipe server creation");
                                    thread::sleep(Duration::from_millis(50));
                                    continue;
                                }
                            },
                        };

                        match connect_named_pipe_with_stop(&listener, &thread_stop) {
                            Ok(true) => {
                                // Pre-create the next instance immediately to avoid connection gaps.
                                next_pipe = create_named_pipe_server(
                                    &pipe_name,
                                    sddl.as_deref(),
                                    MAX_CONTROL_PIPE_INSTANCES,
                                    false,
                                )
                                .ok();

                                if thread_workers.load(Ordering::Acquire)
                                    >= MAX_CONCURRENT_CLIENTS
                                {
                                    handle_client_connection(
                                        listener,
                                        &managed,
                                        &thread_stop,
                                    );
                                } else {
                                    spawn_client_worker(
                                        listener,
                                        Arc::clone(&managed),
                                        Arc::clone(&thread_stop),
                                        Arc::clone(&thread_workers),
                                    );
                                }
                            }
                            Ok(false) => {
                                drop(listener);
                                break;
                            }
                            Err(err) => {
                                tracing::warn!(%err, "ConnectNamedPipe failed on control pipe");
                                drop(listener);
                                thread::sleep(Duration::from_millis(50));
                            }
                        }
                    }
                })
                .map_err(|err| format!("Failed to spawn control pipe thread: {err}"))?;

            Ok(Self {
                stop_flag,
                active_workers,
                worker: Some(worker),
            })
        }

        /// Stops the control pipe server and waits for acceptor and active worker threads to finish.
        pub fn stop_and_join(mut self) {
            self.stop_inner();
        }

        fn stop_inner(&mut self) {
            self.stop_flag.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
            for _ in 0..50 {
                if self.active_workers.load(Ordering::Acquire) == 0 {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
    }

    impl Drop for ControlPipeServer {
        fn drop(&mut self) {
            self.stop_inner();
        }
    }

    fn spawn_client_worker(
        pipe: NamedPipeStream,
        managed: Arc<Mutex<ManagedServer>>,
        stop_flag: Arc<AtomicBool>,
        active_workers: Arc<AtomicUsize>,
    ) {
        active_workers.fetch_add(1, Ordering::AcqRel);
        let thread_workers = Arc::clone(&active_workers);
        let spawn_res = thread::Builder::new()
            .name("touchpad-control-client".to_string())
            .spawn(move || {
                handle_client_connection(pipe, &managed, &stop_flag);
                thread_workers.fetch_sub(1, Ordering::AcqRel);
            });
        if spawn_res.is_err() {
            active_workers.fetch_sub(1, Ordering::AcqRel);
        }
    }

    fn handle_client_connection(
        mut pipe: NamedPipeStream,
        managed: &Arc<Mutex<ManagedServer>>,
        stop_flag: &AtomicBool,
    ) {
        let response = match pipe.read_json_line::<ControlRequest>(
            CONTROL_READ_TIMEOUT_MS,
            Some(stop_flag),
        ) {
            Ok(req) => managed
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .handle_request(req),
            Err(err) => ControlResponse::Error(err),
        };

        if let Err(err) = pipe.write_json_line(&response, CONTROL_WRITE_TIMEOUT_MS) {
            tracing::debug!(%err, "Failed to write control response");
            return;
        }
        // The client closes its handle after reading the response. Wait for that
        // close with a deadline so an idle client cannot block the acceptor.
        let _ = pipe.read_line_timeout(CONTROL_READ_TIMEOUT_MS, Some(stop_flag));
    }
}

#[cfg(windows)]
pub use win_server::ControlPipeServer;
