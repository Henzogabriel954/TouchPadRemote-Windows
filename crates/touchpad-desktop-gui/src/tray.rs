//! Cross-platform system tray controller (`tray-icon` on Windows, no-op fallback on Linux).

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    OpenWindow,
    ToggleServer,
    Quit,
}

#[cfg(windows)]
mod platform {
    use std::sync::mpsc::{self, Receiver};

    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

    use super::TrayAction;
    use crate::icon::generate_app_icon_rgba;

    pub struct TrayController {
        _tray_icon: Option<TrayIcon>,
        toggle_item: Option<MenuItem>,
        rx: Option<Receiver<TrayAction>>,
        last_running: Option<bool>,
    }

    impl TrayController {
        pub fn new(egui_ctx: egui::Context) -> Self {
            match Self::try_init(egui_ctx) {
                Ok(controller) => controller,
                Err(err) => {
                    tracing::warn!(%err, "Não foi possível criar ícone na bandeja do sistema");
                    Self {
                        _tray_icon: None,
                        toggle_item: None,
                        rx: None,
                        last_running: None,
                    }
                }
            }
        }

        fn try_init(egui_ctx: egui::Context) -> Result<Self, String> {
            let icon_rgba = generate_app_icon_rgba(32);
            let tray_img = Icon::from_rgba(icon_rgba, 32, 32)
                .map_err(|err| format!("Ícone inválido para bandeja: {err}"))?;

            let open_item = MenuItem::new("Abrir TouchPad Remote", true, None);
            let toggle_item = MenuItem::new("Parar Servidor", true, None);
            let separator = PredefinedMenuItem::separator();
            let quit_item = MenuItem::new("Sair", true, None);

            let menu = Menu::new();
            menu.append_items(&[&open_item, &toggle_item, &separator, &quit_item])
                .map_err(|err| format!("Falha ao montar menu da bandeja: {err}"))?;

            let (tx, rx) = mpsc::channel::<TrayAction>();

            let open_id = open_item.id().clone();
            let toggle_id = toggle_item.id().clone();
            let quit_id = quit_item.id().clone();
            let menu_tx = tx.clone();
            let menu_ctx = egui_ctx.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let action = if event.id == open_id {
                    Some(TrayAction::OpenWindow)
                } else if event.id == toggle_id {
                    Some(TrayAction::ToggleServer)
                } else if event.id == quit_id {
                    Some(TrayAction::Quit)
                } else {
                    None
                };
                if let Some(action) = action {
                    let _ = menu_tx.send(action);
                    menu_ctx.request_repaint();
                }
            }));

            let icon_tx = tx;
            let icon_ctx = egui_ctx;
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                let open = matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } | TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    }
                );
                if open {
                    let _ = icon_tx.send(TrayAction::OpenWindow);
                    icon_ctx.request_repaint();
                }
            }));

            let tray_icon = TrayIconBuilder::new()
                .with_tooltip("TouchPad Remote")
                .with_icon(tray_img)
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()
                .map_err(|err| format!("TrayIconBuilder falhou: {err}"))?;

            Ok(Self {
                _tray_icon: Some(tray_icon),
                toggle_item: Some(toggle_item),
                rx: Some(rx),
                last_running: None,
            })
        }

        pub fn is_available(&self) -> bool {
            self._tray_icon.is_some()
        }

        pub fn poll_actions(&self) -> Vec<TrayAction> {
            let mut actions = Vec::new();
            if let Some(rx) = &self.rx {
                while let Ok(action) = rx.try_recv() {
                    actions.push(action);
                }
            }
            actions
        }

        pub fn set_server_running(&mut self, running: bool) {
            if self.last_running == Some(running) {
                return;
            }
            self.last_running = Some(running);
            if let Some(item) = &self.toggle_item {
                item.set_text(if running {
                    "Parar Servidor"
                } else {
                    "Iniciar Servidor"
                });
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::TrayAction;

    pub struct TrayController;

    impl TrayController {
        pub fn new(_egui_ctx: egui::Context) -> Self {
            Self
        }

        pub fn is_available(&self) -> bool {
            false
        }

        pub fn poll_actions(&self) -> Vec<TrayAction> {
            Vec::new()
        }

        pub fn set_server_running(&mut self, _running: bool) {}
    }
}

pub use platform::TrayController;
