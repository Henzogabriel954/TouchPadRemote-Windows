//! Pure formatting, badge selection, port parsing, and path helpers for the GUI.

#[cfg(any(windows, test))]
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use touchpad_protocol::{HostOs, MouseButton, PrivilegeMode, ScreenEdge, TouchpadPacket};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ActiveEdges {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeTone {
    Emerald,
    Cyan,
    Amber,
    Red,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrivilegeBadge {
    pub label: &'static str,
    pub tone: BadgeTone,
}

pub fn privilege_badge(
    host_os: HostOs,
    is_service_active: bool,
    privilege: PrivilegeMode,
    linux_unavailable: bool,
) -> PrivilegeBadge {
    match host_os {
        HostOs::Windows => {
            if is_service_active || privilege == PrivilegeMode::System {
                PrivilegeBadge {
                    label: "Serviço UAC Ativo",
                    tone: BadgeTone::Emerald,
                }
            } else if privilege == PrivilegeMode::Admin {
                PrivilegeBadge {
                    label: "Admin Elevado",
                    tone: BadgeTone::Cyan,
                }
            } else {
                PrivilegeBadge {
                    label: "Portátil (Usuário)",
                    tone: BadgeTone::Amber,
                }
            }
        }
        HostOs::Linux => {
            if linux_unavailable {
                PrivilegeBadge {
                    label: "Linux Indisponível",
                    tone: BadgeTone::Red,
                }
            } else if privilege == PrivilegeMode::Uinput {
                PrivilegeBadge {
                    label: "Linux uinput",
                    tone: BadgeTone::Emerald,
                }
            } else if privilege == PrivilegeMode::Xtest {
                PrivilegeBadge {
                    label: "Linux XTest",
                    tone: BadgeTone::Cyan,
                }
            } else {
                PrivilegeBadge {
                    label: "Linux (Usuário)",
                    tone: BadgeTone::Amber,
                }
            }
        }
    }
}

pub fn describe_packet_pt_br(packet: &TouchpadPacket) -> String {
    match packet {
        TouchpadPacket::Move { .. } => "Mover cursor".to_string(),
        TouchpadPacket::Click { button } => match button {
            MouseButton::Left => "Clique esquerdo".to_string(),
            MouseButton::Right => "Clique direito".to_string(),
            MouseButton::Middle => "Clique do meio".to_string(),
        },
        TouchpadPacket::ButtonState { button, pressed } => {
            let action = if *pressed { "Segurar" } else { "Soltar" };
            let btn = match button {
                MouseButton::Left => "esquerdo",
                MouseButton::Right => "direito",
                MouseButton::Middle => "do meio",
            };
            format!("{action} botão {btn}")
        }
        TouchpadPacket::Scroll { delta_y } => {
            if *delta_y >= 0.0 {
                "Rolagem para baixo".to_string()
            } else {
                "Rolagem para cima".to_string()
            }
        }
        TouchpadPacket::DiscoverRequest { .. } => "Solicitação de descoberta".to_string(),
        TouchpadPacket::DiscoverResponse { .. } => "Resposta de descoberta".to_string(),
        TouchpadPacket::Ping { .. } => "Ping".to_string(),
        TouchpadPacket::Pong { .. } => "Pong".to_string(),
        TouchpadPacket::PairRequest { .. } => "Pedido de pareamento".to_string(),
        TouchpadPacket::PairResponse { .. } => "Resposta de pareamento".to_string(),
        TouchpadPacket::EdgeConfig { .. } => "Bordas Multi-PC atualizadas".to_string(),
        TouchpadPacket::EdgeHit { edge, .. } => {
            format!("Borda atingida ({})", edge_label_pt_br(*edge))
        }
        TouchpadPacket::CursorEnter { edge, .. } => {
            format!("Cursor entrou ({})", edge_label_pt_br(*edge))
        }
        TouchpadPacket::CursorLeave { edge } => {
            format!("Cursor saiu ({})", edge_label_pt_br(*edge))
        }
        TouchpadPacket::HostInfo { .. } => "Informações do host".to_string(),
    }
}

pub fn edge_label_pt_br(edge: ScreenEdge) -> &'static str {
    match edge {
        ScreenEdge::Left => "Esquerda",
        ScreenEdge::Right => "Direita",
        ScreenEdge::Top => "Cima",
        ScreenEdge::Bottom => "Baixo",
    }
}

pub fn current_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn format_last_seen_pt_br(timestamp_ms: u64, now_ms: u64) -> String {
    if timestamp_ms == 0 {
        return "Nunca".to_string();
    }
    let elapsed_secs = now_ms.saturating_sub(timestamp_ms) / 1000;
    if elapsed_secs < 60 {
        return "Agora".to_string();
    }
    let mins = elapsed_secs / 60;
    if mins < 60 {
        return format!("há {mins} min");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("há {hours} h");
    }
    let days = hours / 24;
    if days == 1 {
        "há 1 dia".to_string()
    } else {
        format!("há {days} dias")
    }
}

pub fn parse_udp_port(raw: &str) -> Result<u16, &'static str> {
    let trimmed = raw.trim();
    match trimmed.parse::<u16>() {
        Ok(port) if port > 0 => Ok(port),
        _ => Err("Informe uma porta UDP válida entre 1 e 65535."),
    }
}

#[cfg(any(windows, test))]
pub fn resolve_service_exe_path(current_exe: &Path) -> Option<PathBuf> {
    let candidate = current_exe.parent()?.join("touchpad-win-service.exe");
    candidate.is_file().then_some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_expected_privilege_badges() {
        assert_eq!(
            privilege_badge(HostOs::Windows, true, PrivilegeMode::User, false).label,
            "Serviço UAC Ativo"
        );
        assert_eq!(
            privilege_badge(HostOs::Windows, false, PrivilegeMode::Admin, false).label,
            "Admin Elevado"
        );
        assert_eq!(
            privilege_badge(HostOs::Windows, false, PrivilegeMode::User, false).label,
            "Portátil (Usuário)"
        );
        assert_eq!(
            privilege_badge(HostOs::Linux, false, PrivilegeMode::Uinput, false).label,
            "Linux uinput"
        );
        assert_eq!(
            privilege_badge(HostOs::Linux, false, PrivilegeMode::Xtest, false).label,
            "Linux XTest"
        );
        assert_eq!(
            privilege_badge(HostOs::Linux, false, PrivilegeMode::User, true).label,
            "Linux Indisponível"
        );
    }

    #[test]
    fn describes_packets_in_pt_br() {
        assert_eq!(
            describe_packet_pt_br(&TouchpadPacket::Move { dx: 1.0, dy: -1.0 }),
            "Mover cursor"
        );
        assert_eq!(
            describe_packet_pt_br(&TouchpadPacket::Click {
                button: MouseButton::Right
            }),
            "Clique direito"
        );
        assert_eq!(
            describe_packet_pt_br(&TouchpadPacket::ButtonState {
                button: MouseButton::Left,
                pressed: true
            }),
            "Segurar botão esquerdo"
        );
        assert_eq!(
            describe_packet_pt_br(&TouchpadPacket::Scroll { delta_y: -2.0 }),
            "Rolagem para cima"
        );
        assert_eq!(
            describe_packet_pt_br(&TouchpadPacket::EdgeHit {
                edge: ScreenEdge::Right,
                ratio: 0.5
            }),
            "Borda atingida (Direita)"
        );
    }

    #[test]
    fn formats_last_seen_timestamps() {
        let now = 1_700_000_000_000_u64;
        assert_eq!(format_last_seen_pt_br(0, now), "Nunca");
        assert_eq!(format_last_seen_pt_br(now - 20_000, now), "Agora");
        assert_eq!(
            format_last_seen_pt_br(now - 5 * 60_000, now),
            "há 5 min"
        );
        assert_eq!(
            format_last_seen_pt_br(now - 3 * 3_600_000, now),
            "há 3 h"
        );
        assert_eq!(
            format_last_seen_pt_br(now - 24 * 3_600_000, now),
            "há 1 dia"
        );
        assert_eq!(
            format_last_seen_pt_br(now - 3 * 24 * 3_600_000, now),
            "há 3 dias"
        );
    }

    #[test]
    fn parses_udp_ports_and_resolves_service_exe() {
        assert_eq!(parse_udp_port(" 45454 "), Ok(45454));
        assert!(parse_udp_port("0").is_err());
        assert!(parse_udp_port("70000").is_err());
        assert!(parse_udp_port("abc").is_err());

        let temp_root = std::env::temp_dir().join(format!(
            "touchpad-gui-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&temp_root);
        std::fs::create_dir_all(&temp_root).unwrap();
        let gui_exe = temp_root.join("TouchPadRemote.exe");
        assert!(resolve_service_exe_path(&gui_exe).is_none());

        let svc_exe = temp_root.join("touchpad-win-service.exe");
        std::fs::write(&svc_exe, b"stub").unwrap();
        assert_eq!(resolve_service_exe_path(&gui_exe), Some(svc_exe));
        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
