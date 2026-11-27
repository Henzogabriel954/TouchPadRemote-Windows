//! Native `eframe` / `egui` application UI for TouchPad Remote.

use std::collections::HashSet;
use std::net::IpAddr;
use std::time::{Duration, Instant};

use egui::{Align, Color32, CornerRadius, Layout, RichText, Slider, Stroke, TextEdit, ViewportCommand};
use touchpad_server_core::get_local_ip_addresses;
use touchpad_storage::{
    DesktopConfig, MAX_SENSITIVITY, MIN_SENSITIVITY,
};

use crate::client_bridge::{BridgeMode, BridgeState, ClientBridge};
use crate::telemetry::{
    current_epoch_ms, format_last_seen_pt_br, parse_udp_port, privilege_badge, ActiveEdges,
};
use crate::theme::{
    action_button, card_frame, pill_badge, tile_frame, tone_colors, ACCENT_CYAN, ACCENT_CYAN_BG,
    ACCENT_CYAN_BORDER, BG_CANVAS, BG_ELEVATED, BG_SURFACE, BORDER_SUBTLE, STATUS_AMBER,
    STATUS_EMERALD, STATUS_EMERALD_BG, STATUS_EMERALD_BTN_BG, STATUS_EMERALD_TEXT, STATUS_STOPPED,
    STATUS_STOPPED_BG, STATUS_STOPPED_BTN_BG, STATUS_STOPPED_TEXT, TEXT_MUTED, TEXT_PRIMARY,
    TEXT_SECONDARY,
};
use crate::tray::{TrayAction, TrayController};

pub struct TouchPadGuiApp {
    bridge: ClientBridge,
    tray: TrayController,
    local_ips: Vec<String>,
    selected_primary_ip: Option<String>,
    copied_until: Option<Instant>,
    draft_sensitivity: f32,
    draft_scroll_sensitivity: f32,
    draft_edge_overshoot_px: u16,
    draft_require_pairing_approval: bool,
    draft_start_minimized_to_tray: bool,
    draft_port_input: String,
    port_validation_error: Option<String>,
    synced_config_revision: u64,
    known_pending_keys: HashSet<(String, IpAddr)>,
    quitting: bool,
    window_hidden: bool,
    initial_hide_frames: u8,
    #[cfg(windows)]
    relaunch_error: Option<String>,
}

impl TouchPadGuiApp {
    pub fn new(egui_ctx: egui::Context, start_minimized: bool) -> Self {
        let bridge = ClientBridge::start(egui_ctx.clone());
        let tray = TrayController::new(egui_ctx);
        let state = bridge.state();
        let cfg = &state.snapshot.config;
        let local_ips = get_local_ip_addresses();
        let selected_primary_ip = local_ips.first().cloned();
        let hide_on_boot = start_minimized && tray.is_available();

        let known_pending_keys = state
            .snapshot
            .pending_pairings
            .iter()
            .map(|p| (p.client_id.clone(), p.ip_address))
            .collect();

        Self {
            bridge,
            tray,
            local_ips,
            selected_primary_ip,
            copied_until: None,
            draft_sensitivity: cfg.sensitivity.clamp(MIN_SENSITIVITY, MAX_SENSITIVITY),
            draft_scroll_sensitivity: cfg.scroll_sensitivity.clamp(0.3, 3.0),
            draft_edge_overshoot_px: cfg.edge_overshoot_px.min(120),
            draft_require_pairing_approval: cfg.require_pairing_approval,
            draft_start_minimized_to_tray: cfg.start_minimized_to_tray,
            draft_port_input: cfg.port.to_string(),
            port_validation_error: None,
            synced_config_revision: state.config_revision,
            known_pending_keys,
            quitting: false,
            window_hidden: hide_on_boot,
            initial_hide_frames: if hide_on_boot { 2 } else { 0 },
            #[cfg(windows)]
            relaunch_error: None,
        }
    }

    fn sync_draft_from_state(&mut self, ctx: &egui::Context, state: &BridgeState, port_focused: bool) {
        let dragging = ctx.dragged_id().is_some();
        if state.config_revision != self.synced_config_revision && !dragging {
            let cfg = &state.snapshot.config;
            self.draft_sensitivity = cfg.sensitivity.clamp(MIN_SENSITIVITY, MAX_SENSITIVITY);
            self.draft_scroll_sensitivity = cfg.scroll_sensitivity.clamp(0.3, 3.0);
            self.draft_edge_overshoot_px = cfg.edge_overshoot_px.min(120);
            self.draft_require_pairing_approval = cfg.require_pairing_approval;
            self.draft_start_minimized_to_tray = cfg.start_minimized_to_tray;
            if !port_focused {
                self.draft_port_input = cfg.port.to_string();
                self.port_validation_error = None;
            }
            self.synced_config_revision = state.config_revision;
        }
    }

    fn commit_settings(&mut self, base: &DesktopConfig, new_port: Option<u16>) {
        let mut next = base.clone();
        next.sensitivity = self.draft_sensitivity.clamp(MIN_SENSITIVITY, MAX_SENSITIVITY);
        next.scroll_sensitivity = self.draft_scroll_sensitivity.clamp(0.3, 3.0);
        next.edge_overshoot_px = self.draft_edge_overshoot_px.min(120);
        next.require_pairing_approval = self.draft_require_pairing_approval;
        next.start_minimized_to_tray = self.draft_start_minimized_to_tray;
        if let Some(port) = new_port {
            next.port = port;
        }
        self.bridge.update_config(next);
    }

    fn refresh_network_info(&mut self) {
        self.local_ips = get_local_ip_addresses();
        if let Some(selected) = &self.selected_primary_ip {
            if !self.local_ips.contains(selected) {
                self.selected_primary_ip = self.local_ips.first().cloned();
            }
        } else {
            self.selected_primary_ip = self.local_ips.first().cloned();
        }
    }

    fn copy_ip(&mut self, ctx: &egui::Context, ip: &str) {
        if ip.is_empty() {
            return;
        }
        ctx.copy_text(ip.to_string());
        self.copied_until = Some(Instant::now() + Duration::from_millis(1800));
    }

    fn handle_tray_and_lifecycle(&mut self, ctx: &egui::Context, state: &BridgeState) {
        if self.initial_hide_frames > 0 {
            self.initial_hide_frames -= 1;
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
        }

        for action in self.tray.poll_actions() {
            match action {
                TrayAction::OpenWindow => {
                    self.window_hidden = false;
                    self.initial_hide_frames = 0;
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                TrayAction::ToggleServer => {
                    if state.snapshot.running {
                        self.bridge.stop_server();
                    } else {
                        self.bridge.start_server();
                    }
                }
                TrayAction::Quit => {
                    self.quitting = true;
                    self.bridge.shutdown();
                    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }

        self.tray.set_server_running(state.snapshot.running);

        let current_pending: HashSet<(String, IpAddr)> = state
            .snapshot
            .pending_pairings
            .iter()
            .map(|p| (p.client_id.clone(), p.ip_address))
            .collect();
        let has_new_pending = current_pending
            .iter()
            .any(|key| !self.known_pending_keys.contains(key));
        if has_new_pending {
            self.window_hidden = false;
            self.initial_hide_frames = 0;
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(ViewportCommand::Focus);
            ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(
                egui::UserAttentionType::Critical,
            ));
        }
        self.known_pending_keys = current_pending;

        if ctx.input(|i| i.viewport().close_requested()) {
            if !self.quitting && self.tray.is_available() && self.draft_start_minimized_to_tray {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(ViewportCommand::Visible(false));
                self.window_hidden = true;
            } else {
                self.quitting = true;
                self.bridge.shutdown();
            }
        }
    }

    fn render_header(&self, ui: &mut egui::Ui, state: &BridgeState) {
        let linux_unavailable = state
            .linux_status
            .as_ref()
            .is_some_and(|diag| diag.unavailable_error.is_some());
        let priv_badge = privilege_badge(
            state.host_os,
            state.service_running,
            state.privilege_mode,
            linux_unavailable,
        );

        ui.horizontal_wrapped(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new("TouchPad Remote")
                        .size(22.0)
                        .strong()
                        .color(TEXT_PRIMARY),
                );
                ui.label(
                    RichText::new("Use seu celular como touchpad sem fio na mesma rede local")
                        .size(13.0)
                        .color(TEXT_SECONDARY),
                );
            });

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (p_bg, p_border, p_text, _) = tone_colors(priv_badge.tone);
                pill_badge(ui, priv_badge.label, None, p_bg, p_border, p_text);

                if state.snapshot.running {
                    pill_badge(
                        ui,
                        "Ativo na Rede",
                        Some(STATUS_EMERALD),
                        STATUS_EMERALD_BG,
                        STATUS_EMERALD,
                        STATUS_EMERALD_TEXT,
                    );
                } else {
                    pill_badge(
                        ui,
                        "Parado",
                        Some(STATUS_STOPPED),
                        STATUS_STOPPED_BG,
                        STATUS_STOPPED,
                        STATUS_STOPPED_TEXT,
                    );
                }
            });
        });
    }

    #[cfg(windows)]
    fn render_windows_uac_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, state: &BridgeState) {
        let border = if state.service_running {
            STATUS_EMERALD
        } else {
            BORDER_SUBTLE
        };
        card_frame(border, BG_SURFACE).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Permissões Windows e Serviço UAC")
                        .size(14.5)
                        .strong()
                        .color(TEXT_PRIMARY),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let status_text = if state.service_running {
                        ("Serviço LocalSystem em execução", STATUS_EMERALD)
                    } else if state.service_installed {
                        ("Serviço instalado (parado)", STATUS_AMBER)
                    } else {
                        ("Modo portátil (sem privilégio UAC)", TEXT_MUTED)
                    };
                    ui.label(
                        RichText::new(status_text.0)
                            .size(12.0)
                            .strong()
                            .color(status_text.1),
                    );
                });
            });

            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "No Windows, o Gerenciador de Tarefas e as telas \"Sim/Não\" do UAC rodam na Área de Trabalho Segura e bloqueiam apps comuns. Instale o Serviço UAC (LocalSystem) ou reinicie como Administrador para controlar essas janelas pelo celular.",
                )
                .size(12.5)
                .color(TEXT_SECONDARY),
            );
            ui.add_space(8.0);

            let busy = state.service_transition_busy;
            ui.horizontal_wrapped(|ui| {
                if !state.service_installed {
                    let install_btn = action_button(
                        "Instalar Serviço UAC (Sim/Não + Gerenciador)",
                        STATUS_EMERALD_BTN_BG,
                        STATUS_EMERALD,
                        STATUS_EMERALD_TEXT,
                    );
                    if ui.add_enabled(!busy, install_btn).clicked() {
                        self.bridge.install_windows_service();
                    }
                } else {
                    if !state.service_running {
                        let start_svc_btn = action_button(
                            "Reinstalar / Iniciar Serviço UAC",
                            STATUS_EMERALD_BTN_BG,
                            STATUS_EMERALD,
                            STATUS_EMERALD_TEXT,
                        );
                        if ui.add_enabled(!busy, start_svc_btn).clicked() {
                            self.bridge.install_windows_service();
                        }
                    }

                    let uninstall_btn = action_button(
                        "Desinstalar Serviço UAC",
                        STATUS_STOPPED_BTN_BG,
                        STATUS_STOPPED,
                        STATUS_STOPPED_TEXT,
                    );
                    if ui.add_enabled(!busy, uninstall_btn).clicked() {
                        self.bridge.uninstall_windows_service();
                    }
                }

                if !touchpad_input_win::is_process_elevated() && !state.service_running {
                    let admin_btn = action_button(
                        "Reiniciar como Administrador",
                        ACCENT_CYAN_BG,
                        ACCENT_CYAN_BORDER,
                        ACCENT_CYAN,
                    );
                    if ui.add_enabled(!busy, admin_btn).clicked() {
                        self.relaunch_error = None;
                        if let Err(err) = self.bridge.prepare_relaunch_as_admin() {
                            self.relaunch_error = Some(err);
                        } else {
                            match touchpad_input_win::relaunch_as_admin(&[]) {
                                Ok(()) => {
                                    self.quitting = true;
                                    self.bridge.shutdown();
                                    ctx.send_viewport_cmd(ViewportCommand::Close);
                                }
                                Err(err) => {
                                    self.bridge.start_server();
                                    self.relaunch_error = Some(format!(
                                        "Não foi possível reiniciar como Administrador: {err}"
                                    ));
                                }
                            }
                        }
                    }
                }
            });

            if let Some(notice) = &state.service_status_notice {
                ui.add_space(6.0);
                ui.label(RichText::new(notice).size(12.0).color(ACCENT_CYAN));
            }
            if let Some(err) = &self.relaunch_error {
                ui.add_space(4.0);
                ui.label(RichText::new(err).size(12.0).color(STATUS_STOPPED));
            }
        });
    }

    #[cfg(target_os = "linux")]
    fn render_linux_diagnostic_card(&self, ui: &mut egui::Ui, state: &BridgeState) {
        use crate::theme::{STATUS_AMBER_BG, STATUS_AMBER_TEXT};

        let Some(diag) = &state.linux_status else {
            return;
        };
        let has_warning = diag.hint.is_some() || diag.unavailable_error.is_some();
        let (fill, border, text_color) = if diag.unavailable_error.is_some() {
            (STATUS_STOPPED_BG, STATUS_STOPPED, STATUS_STOPPED_TEXT)
        } else if diag.hint.is_some() {
            (STATUS_AMBER_BG, STATUS_AMBER, STATUS_AMBER_TEXT)
        } else {
            (BG_SURFACE, BORDER_SUBTLE, TEXT_SECONDARY)
        };

        card_frame(border, fill).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Entrada do Sistema (Linux)")
                        .size(14.5)
                        .strong()
                        .color(TEXT_PRIMARY),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "Sessão: {} · Injeção: {}",
                            diag.session_label, diag.injection_label
                        ))
                        .size(12.0)
                        .strong()
                        .color(if has_warning { text_color } else { ACCENT_CYAN }),
                    );
                });
            });

            if let Some(hint) = &diag.hint {
                ui.add_space(6.0);
                ui.label(RichText::new(hint).size(12.5).color(text_color));
            }
            if let Some(err) = &diag.unavailable_error {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!("Detalhes: {err}"))
                        .size(12.0)
                        .color(STATUS_STOPPED_TEXT),
                );
            }
        });
    }

    fn render_network_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, state: &BridgeState) {
        card_frame(BORDER_SUBTLE, BG_SURFACE).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Dados para Cadastrar no Celular")
                        .size(14.5)
                        .strong()
                        .color(TEXT_PRIMARY),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(action_button(
                            "Atualizar Rede",
                            BG_ELEVATED,
                            BORDER_SUBTLE,
                            TEXT_SECONDARY,
                        ))
                        .clicked()
                    {
                        self.refresh_network_info();
                    }
                });
            });

            ui.add_space(8.0);
            let primary_ip = self
                .selected_primary_ip
                .clone()
                .or_else(|| self.local_ips.first().cloned())
                .unwrap_or_else(|| "Nenhum IP local detectado".to_string());
            let has_ip = !self.local_ips.is_empty();
            let copied_active = self.copied_until.is_some_and(|t| Instant::now() < t);

            tile_frame(ACCENT_CYAN_BORDER).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Endereço IP na Rede Local")
                                .size(12.0)
                                .color(TEXT_SECONDARY),
                        );
                        ui.label(
                            RichText::new(&primary_ip)
                                .monospace()
                                .size(22.0)
                                .strong()
                                .color(ACCENT_CYAN),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let label = if copied_active { "Copiado!" } else { "Copiar IP" };
                        let btn = action_button(label, BG_SURFACE, ACCENT_CYAN_BORDER, ACCENT_CYAN);
                        if ui.add_enabled(has_ip, btn).clicked() {
                            self.copy_ip(ctx, &primary_ip);
                        }
                    });
                });

                if self.local_ips.len() > 1 {
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("Outros IPs:").size(11.5).color(TEXT_MUTED));
                        let extra: Vec<String> = self
                            .local_ips
                            .iter()
                            .filter(|ip| *ip != &primary_ip)
                            .take(5)
                            .cloned()
                            .collect();
                        for ip in extra {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(&ip)
                                            .monospace()
                                            .size(11.5)
                                            .color(TEXT_SECONDARY),
                                    )
                                    .fill(BG_SURFACE)
                                    .stroke(Stroke::new(1.0_f32, BORDER_SUBTLE))
                                    .corner_radius(CornerRadius::same(7)),
                                )
                                .on_hover_text("Clique para usar e copiar este IP")
                                .clicked()
                            {
                                self.selected_primary_ip = Some(ip.clone());
                                self.copy_ip(ctx, &ip);
                            }
                        }
                    });
                } else if self.local_ips.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "Conecte o computador ao Wi-Fi ou cabo Ethernet e clique em Atualizar Rede.",
                        )
                        .size(11.5)
                        .color(STATUS_AMBER),
                    );
                }
            });

            ui.add_space(8.0);
            ui.columns(2, |cols| {
                cols[0].vertical(|ui| {
                    tile_frame(BORDER_SUBTLE).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new("Nome do Computador")
                                .size(12.0)
                                .color(TEXT_SECONDARY),
                        );
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(&state.snapshot.config.computer_name)
                                .size(14.0)
                                .strong()
                                .color(TEXT_PRIMARY),
                        );
                    });
                });
                cols[1].vertical(|ui| {
                    let port = if state.snapshot.running {
                        state.snapshot.actual_port
                    } else {
                        state.snapshot.config.port
                    };
                    tile_frame(BORDER_SUBTLE).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("Porta UDP").size(12.0).color(TEXT_SECONDARY));
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(port.to_string())
                                .monospace()
                                .size(15.0)
                                .strong()
                                .color(TEXT_PRIMARY),
                        );
                    });
                });
            });
        });
    }

    fn render_pairing_and_devices_card(&self, ui: &mut egui::Ui, state: &BridgeState) {
        let pending = &state.snapshot.pending_pairings;
        let saved_devices = &state.snapshot.config.allowed_devices;
        let approved_clients = &state.snapshot.approved_clients;

        let (card_fill, card_border, status_text, status_color) = if !pending.is_empty() {
            let txt = if pending.len() == 1 {
                "Solicitação pendente".to_string()
            } else {
                format!("{} solicitações pendentes", pending.len())
            };
            (ACCENT_CYAN_BG, ACCENT_CYAN, txt, ACCENT_CYAN)
        } else if !saved_devices.is_empty() {
            let txt = if saved_devices.len() == 1 {
                "1 dispositivo salvo".to_string()
            } else {
                format!("{} dispositivos salvos", saved_devices.len())
            };
            (BG_SURFACE, STATUS_EMERALD, txt, STATUS_EMERALD)
        } else {
            (BG_SURFACE, BORDER_SUBTLE, "Aguardando".to_string(), TEXT_MUTED)
        };

        card_frame(card_border, card_fill).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Pareamento e Dispositivos Permitidos Salvos")
                        .size(14.5)
                        .strong()
                        .color(TEXT_PRIMARY),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(status_text)
                            .size(12.0)
                            .strong()
                            .color(status_color),
                    );
                });
            });

            ui.add_space(8.0);

            if !pending.is_empty() {
                for client in pending {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(0x16, 0x26, 0x3D))
                        .stroke(Stroke::new(1.2_f32, ACCENT_CYAN))
                        .corner_radius(CornerRadius::same(11))
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(
                                        RichText::new("Solicitação de pareamento recebida")
                                            .size(11.5)
                                            .strong()
                                            .color(ACCENT_CYAN),
                                    );
                                    ui.label(
                                        RichText::new(&client.client_name)
                                            .size(14.5)
                                            .strong()
                                            .color(TEXT_PRIMARY),
                                    );
                                    ui.label(
                                        RichText::new(format!("IP do celular: {}", client.ip_address))
                                            .monospace()
                                            .size(12.0)
                                            .color(TEXT_SECONDARY),
                                    );
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui
                                        .add(action_button(
                                            "Recusar",
                                            STATUS_STOPPED_BTN_BG,
                                            STATUS_STOPPED,
                                            STATUS_STOPPED_TEXT,
                                        ))
                                        .clicked()
                                    {
                                        self.bridge.reject_pairing(client.ip_address);
                                    }
                                    if ui
                                        .add(action_button(
                                            "Aceitar Pareamento",
                                            STATUS_EMERALD_BTN_BG,
                                            STATUS_EMERALD,
                                            STATUS_EMERALD_TEXT,
                                        ))
                                        .clicked()
                                    {
                                        self.bridge.approve_pairing(client.ip_address);
                                    }
                                });
                            });
                        });
                    ui.add_space(6.0);
                }
            }

            if saved_devices.is_empty() {
                if pending.is_empty() {
                    tile_frame(BORDER_SUBTLE).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new(
                                "Nenhum celular salvo ainda. Quando você tocar no quadrado do aplicativo no celular, o pedido aparecerá aqui; após aceitar, o celular reconectará automaticamente.",
                            )
                            .size(12.5)
                            .color(TEXT_SECONDARY),
                        );
                    });
                }
            } else {
                ui.label(
                    RichText::new(
                        "Celulares salvos abaixo são autorizados automaticamente ao reconectar.",
                    )
                    .size(12.0)
                    .color(STATUS_EMERALD),
                );
                ui.add_space(4.0);

                let now_ms = current_epoch_ms();
                for device in saved_devices {
                    let is_online_approved = approved_clients.iter().any(|client| {
                        (!device.client_id.is_empty() && client.client_id == device.client_id)
                            || (!device.last_ip.is_empty()
                                && client.ip_address.to_string() == device.last_ip)
                    });
                    let row_border = if is_online_approved {
                        Color32::from_rgb(0x16, 0x5B, 0x45)
                    } else {
                        BORDER_SUBTLE
                    };
                    tile_frame(row_border).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (dot_rect, _) =
                                ui.allocate_exact_size(egui::Vec2::splat(12.0), egui::Sense::hover());
                            ui.painter().circle_filled(
                                dot_rect.center(),
                                4.5,
                                if is_online_approved {
                                    STATUS_EMERALD
                                } else {
                                    TEXT_MUTED
                                },
                            );

                            ui.vertical(|ui| {
                                let name = if device.client_name.is_empty() {
                                    "Celular salvo"
                                } else {
                                    device.client_name.as_str()
                                };
                                let ip_text = if device.last_ip.is_empty() {
                                    "IP ainda não registrado"
                                } else {
                                    device.last_ip.as_str()
                                };
                                let seen_text = format_last_seen_pt_br(device.last_seen_ms, now_ms);
                                ui.label(
                                    RichText::new(name)
                                        .size(13.5)
                                        .strong()
                                        .color(TEXT_PRIMARY),
                                );
                                ui.label(
                                    RichText::new(format!(
                                        "Último IP: {ip_text} · Visto por último: {seen_text}"
                                    ))
                                    .monospace()
                                    .size(11.5)
                                    .color(TEXT_SECONDARY),
                                );
                            });

                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .add(action_button(
                                        "Revogar",
                                        BG_SURFACE,
                                        STATUS_STOPPED,
                                        STATUS_STOPPED_TEXT,
                                    ))
                                    .clicked()
                                {
                                    self.bridge.revoke_device(
                                        device.client_id.clone(),
                                        device.last_ip.clone(),
                                    );
                                }
                            });
                        });
                    });
                    ui.add_space(4.0);
                }
            }
        });
    }

    fn render_settings_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, state: &BridgeState) -> bool {
        let mut port_has_focus = false;
        let base_cfg = &state.snapshot.config;

        card_frame(BORDER_SUBTLE, BG_SURFACE).show(ui, |ui| {
            ui.label(
                RichText::new("Configurações Salvas")
                    .size(14.5)
                    .strong()
                    .color(TEXT_PRIMARY),
            );
            ui.add_space(8.0);

            let mut should_commit = false;

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Sensibilidade do Cursor")
                        .size(13.0)
                        .color(TEXT_SECONDARY),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{:.1}x", self.draft_sensitivity))
                            .monospace()
                            .strong()
                            .color(ACCENT_CYAN),
                    );
                });
            });
            let sens_resp = ui.add(
                Slider::new(&mut self.draft_sensitivity, MIN_SENSITIVITY..=MAX_SENSITIVITY)
                    .step_by(0.1)
                    .show_value(false),
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new("0.5x (Preciso)").size(11.0).color(TEXT_MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new("4.0x (Rápido)").size(11.0).color(TEXT_MUTED));
                });
            });
            if sens_resp.drag_stopped() || (sens_resp.changed() && !sens_resp.dragged()) {
                should_commit = true;
            }

            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Sensibilidade da Rolagem")
                        .size(13.0)
                        .color(TEXT_SECONDARY),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{:.1}x", self.draft_scroll_sensitivity))
                            .monospace()
                            .strong()
                            .color(ACCENT_CYAN),
                    );
                });
            });
            let scroll_resp = ui.add(
                Slider::new(&mut self.draft_scroll_sensitivity, 0.3..=3.0)
                    .step_by(0.1)
                    .show_value(false),
            );
            if scroll_resp.drag_stopped() || (scroll_resp.changed() && !scroll_resp.dragged()) {
                should_commit = true;
            }

            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Resistência de Borda Multi-PC")
                        .size(13.0)
                        .color(TEXT_SECONDARY),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{} px", self.draft_edge_overshoot_px))
                            .monospace()
                            .strong()
                            .color(ACCENT_CYAN),
                    );
                });
            });
            let edge_resp = ui.add(
                Slider::new(&mut self.draft_edge_overshoot_px, 0..=120)
                    .step_by(1.0)
                    .show_value(false),
            );
            if edge_resp.drag_stopped() || (edge_resp.changed() && !edge_resp.dragged()) {
                should_commit = true;
            }

            ui.add_space(8.0);

            if ui
                .checkbox(
                    &mut self.draft_require_pairing_approval,
                    "Exigir aprovação de novos celulares",
                )
                .changed()
            {
                should_commit = true;
            }

            let tray_label = if cfg!(windows) {
                "Iniciar minimizado na bandeja"
            } else {
                "Iniciar minimizado na bandeja (no Linux use touchpad-daemon em segundo plano)"
            };
            if ui
                .checkbox(&mut self.draft_start_minimized_to_tray, tray_label)
                .changed()
            {
                should_commit = true;
            }

            if should_commit {
                self.commit_settings(base_cfg, None);
            }

            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Porta UDP:").size(13.0).color(TEXT_SECONDARY));
                let port_resp = ui.add(
                    TextEdit::singleline(&mut self.draft_port_input)
                        .desired_width(84.0)
                        .font(egui::TextStyle::Monospace),
                );
                port_has_focus = port_resp.has_focus();
                let enter_pressed =
                    port_resp.lost_focus() && ctx.input(|i| i.key_pressed(egui::Key::Enter));

                if ui
                    .add(action_button(
                        "Aplicar Porta",
                        BG_ELEVATED,
                        ACCENT_CYAN_BORDER,
                        ACCENT_CYAN,
                    ))
                    .clicked()
                    || enter_pressed
                {
                    match parse_udp_port(&self.draft_port_input) {
                        Ok(port) => {
                            self.port_validation_error = None;
                            self.commit_settings(base_cfg, Some(port));
                        }
                        Err(msg) => {
                            self.port_validation_error = Some(msg.to_string());
                        }
                    }
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if state.snapshot.running {
                        if ui
                            .add(action_button(
                                "Parar Servidor",
                                STATUS_STOPPED_BTN_BG,
                                STATUS_STOPPED,
                                STATUS_STOPPED_TEXT,
                            ))
                            .clicked()
                        {
                            self.bridge.stop_server();
                        }
                    } else if ui
                        .add(action_button(
                            "Iniciar Servidor",
                            STATUS_EMERALD_BTN_BG,
                            STATUS_EMERALD,
                            STATUS_EMERALD_TEXT,
                        ))
                        .clicked()
                    {
                        self.bridge.start_server();
                    }
                });
            });

            if let Some(err) = &self.port_validation_error {
                ui.add_space(4.0);
                ui.label(RichText::new(err).size(12.0).color(STATUS_STOPPED));
            }
            if let Some(err) = &state.last_error {
                ui.add_space(4.0);
                ui.label(RichText::new(err).size(12.0).color(STATUS_STOPPED));
            }
        });

        port_has_focus
    }

    fn render_telemetry_card(&self, ui: &mut egui::Ui, state: &BridgeState) {
        card_frame(BORDER_SUBTLE, BG_SURFACE).show(ui, |ui| {
            ui.label(
                RichText::new("Telemetria em Tempo Real")
                    .size(14.5)
                    .strong()
                    .color(TEXT_PRIMARY),
            );
            ui.add_space(8.0);

            let last_ip = state
                .snapshot
                .last_client_ip
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "Aguardando celular...".to_string());
            let packet_count = state.snapshot.packet_count.to_string();

            ui.columns(3, |cols| {
                cols[0].vertical(|ui| {
                    tile_frame(BORDER_SUBTLE).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new("Último celular conectado")
                                .size(11.5)
                                .color(TEXT_SECONDARY),
                        );
                        ui.add_space(3.0);
                        ui.label(
                            RichText::new(last_ip)
                                .monospace()
                                .size(13.0)
                                .strong()
                                .color(TEXT_PRIMARY),
                        );
                    });
                });
                cols[1].vertical(|ui| {
                    tile_frame(BORDER_SUBTLE).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new("Comandos recebidos")
                                .size(11.5)
                                .color(TEXT_SECONDARY),
                        );
                        ui.add_space(3.0);
                        ui.label(
                            RichText::new(packet_count)
                                .monospace()
                                .size(13.0)
                                .strong()
                                .color(TEXT_PRIMARY),
                        );
                    });
                });
                cols[2].vertical(|ui| {
                    tile_frame(BORDER_SUBTLE).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(
                            RichText::new("Última ação executada")
                                .size(11.5)
                                .color(TEXT_SECONDARY),
                        );
                        ui.add_space(3.0);
                        ui.label(
                            RichText::new(&state.last_action_text)
                                .size(13.0)
                                .strong()
                                .color(TEXT_PRIMARY),
                        );
                    });
                });
            });

            ui.add_space(8.0);
            Self::render_active_edges(ui, state.active_edges);
            if state.mode == BridgeMode::WindowsService {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "No modo Serviço UAC, a detecção de bordas Multi-PC é executada pelo serviço em segundo plano.",
                    )
                    .size(11.5)
                    .color(TEXT_MUTED),
                );
            }
        });
    }

    fn render_active_edges(ui: &mut egui::Ui, edges: ActiveEdges) {
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Bordas Multi-PC ativas:")
                    .size(12.0)
                    .color(TEXT_SECONDARY),
            );
            for (name, active) in [
                ("Esquerda", edges.left),
                ("Direita", edges.right),
                ("Cima", edges.top),
                ("Baixo", edges.bottom),
            ] {
                if active {
                    pill_badge(
                        ui,
                        name,
                        Some(STATUS_EMERALD),
                        STATUS_EMERALD_BG,
                        STATUS_EMERALD,
                        STATUS_EMERALD_TEXT,
                    );
                } else {
                    pill_badge(ui, name, None, BG_ELEVATED, BORDER_SUBTLE, TEXT_MUTED);
                }
            }
        });
    }
}

impl eframe::App for TouchPadGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let state = self.bridge.state();
        self.handle_tray_and_lifecycle(ctx, &state);
        ctx.request_repaint_after(Duration::from_millis(300));

        if self.window_hidden {
            return;
        }

        let mut port_focused = false;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(BG_CANVAS)
                    .inner_margin(egui::Margin::symmetric(20, 16)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        self.render_header(ui, &state);
                        ui.add_space(12.0);

                        #[cfg(windows)]
                        {
                            self.render_windows_uac_card(ui, ctx, &state);
                            ui.add_space(10.0);
                        }

                        #[cfg(target_os = "linux")]
                        {
                            self.render_linux_diagnostic_card(ui, &state);
                            ui.add_space(10.0);
                        }

                        self.render_network_card(ui, ctx, &state);
                        ui.add_space(10.0);

                        self.render_pairing_and_devices_card(ui, &state);
                        ui.add_space(10.0);

                        port_focused = self.render_settings_card(ui, ctx, &state);
                        ui.add_space(10.0);

                        self.render_telemetry_card(ui, &state);
                    });
            });

        self.sync_draft_from_state(ctx, &state, port_focused);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.bridge.shutdown();
    }
}
