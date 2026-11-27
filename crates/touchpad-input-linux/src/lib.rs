//! Linux input injection and portable cursor geometry.

use serde::Deserialize;
use touchpad_server_core::clamp_to_monitors;
pub use touchpad_server_core::MonitorRect;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{LinuxInputBackend, LinuxInputError};

const DEFAULT_MONITOR: MonitorRect = MonitorRect {
    x: 0,
    y: 0,
    width: 1920,
    height: 1080,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    Wayland,
    X11,
    Headless,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionMethod {
    Uinput,
    Xtest,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendStatusInfo {
    pub session: SessionKind,
    pub injection: InjectionMethod,
    pub hint: Option<&'static str>,
}

impl BackendStatusInfo {
    pub fn new(session: SessionKind, injection: InjectionMethod, permission_denied: bool) -> Self {
        Self {
            session,
            injection,
            hint: permission_denied.then_some(
                "Sem permissão para acessar /dev/uinput. Execute install-linux.sh para instalar a regra udev e entre novamente na sessão.",
            ),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VirtualCursorTracker {
    monitors: Vec<MonitorRect>,
    position: (i32, i32),
}

impl VirtualCursorTracker {
    pub fn new(mut monitors: Vec<MonitorRect>, initial_position: (i32, i32)) -> Self {
        monitors.retain(|monitor| monitor.width > 0 && monitor.height > 0);
        if monitors.is_empty() {
            monitors.push(DEFAULT_MONITOR);
        }
        let position = clamp_to_monitors(
            initial_position.0.into(),
            initial_position.1.into(),
            &monitors,
        );
        Self { monitors, position }
    }

    pub fn position(&self) -> (i32, i32) {
        self.position
    }

    pub fn monitor_bounds(&self) -> &[MonitorRect] {
        &self.monitors
    }

    pub fn apply_relative(&mut self, dx: i32, dy: i32) -> (i32, i32) {
        self.position = clamp_to_monitors(
            i64::from(self.position.0) + i64::from(dx),
            i64::from(self.position.1) + i64::from(dy),
            &self.monitors,
        );
        self.position
    }

    pub fn warp(&mut self, x: i32, y: i32) -> (i32, i32) {
        self.position = clamp_to_monitors(x.into(), y.into(), &self.monitors);
        self.position
    }

    /// Relative moves that try to align the pointer with a corner before reaching an edge target.
    /// The compositor can still alter relative motion through acceleration or confinement.
    pub fn corner_sweep(&self, x: i32, y: i32) -> Option<[(i32, i32); 3]> {
        // A corner sweep needs a continuous surface. On multi-output desktops,
        // an intervening gap can block either leg, so use the tracked delta.
        let [monitor] = self.monitors.as_slice() else {
            return None;
        };
        let (x, y) = clamp_to_monitors(x.into(), y.into(), &self.monitors);
        // The server places incoming cursors four pixels inside the desktop edge.
        const EDGE_INSET: i32 = 4;
        let (left, top) = (monitor.x, monitor.y);
        let right = left.saturating_add(monitor.width - 1);
        let bottom = top.saturating_add(monitor.height - 1);
        let sweep_x = monitor.width.saturating_mul(4);
        let sweep_y = monitor.height.saturating_mul(4);

        if x.saturating_sub(left) <= EDGE_INSET {
            Some([(-sweep_x, 0), (0, -sweep_y), (x - left, y - top)])
        } else if right.saturating_sub(x) <= EDGE_INSET {
            Some([(sweep_x, 0), (0, -sweep_y), (x - right, y - top)])
        } else if y.saturating_sub(top) <= EDGE_INSET {
            Some([(0, -sweep_y), (-sweep_x, 0), (x - left, y - top)])
        } else if bottom.saturating_sub(y) <= EDGE_INSET {
            Some([(0, sweep_y), (-sweep_x, 0), (x - left, y - bottom)])
        } else {
            None
        }
    }
}

#[derive(Deserialize)]
struct HyprlandMonitor {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    #[serde(default = "unit_scale")]
    scale: f64,
    #[serde(default)]
    transform: u8,
    #[serde(default)]
    disabled: bool,
}

fn unit_scale() -> f64 {
    1.0
}

pub fn parse_hyprland_monitors_json(json: &str) -> Vec<MonitorRect> {
    let Ok(entries) = serde_json::from_str::<Vec<serde_json::Value>>(json) else {
        return Vec::new();
    };
    entries
        .into_iter()
        .filter_map(|entry| {
            let monitor: HyprlandMonitor = serde_json::from_value(entry).ok()?;
            if monitor.disabled || monitor.width <= 0 || monitor.height <= 0 || monitor.scale <= 0.0
            {
                return None;
            }
            let (width, height) = if monitor.transform % 2 == 1 {
                (monitor.height, monitor.width)
            } else {
                (monitor.width, monitor.height)
            };
            let width = (f64::from(width) / monitor.scale).round() as i32;
            let height = (f64::from(height) / monitor.scale).round() as i32;
            (width > 0 && height > 0).then_some(MonitorRect {
                x: monitor.x,
                y: monitor.y,
                width,
                height,
            })
        })
        .collect()
}

pub fn parse_hyprland_cursorpos_json(json: &str) -> Option<(i32, i32)> {
    #[derive(Deserialize)]
    struct Position {
        x: f64,
        y: f64,
    }

    let pos: Position = serde_json::from_str(json).ok()?;
    let (x, y) = (pos.x.round(), pos.y.round());
    let range = f64::from(i32::MIN)..=f64::from(i32::MAX);
    (range.contains(&x) && range.contains(&y)).then_some((x as i32, y as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scaled_rotated_hyprland_monitors() {
        let json = r#"[
            {"name":"DP-1","width":2560,"height":1440,"scale":2,"x":0,"y":0,"transform":0},
            {"name":"DP-2","width":1920,"height":1080,"scale":1.5,"x":-720,"y":0,"transform":1},
            {"name":"DP-3","width":1920,"height":1080,"x":0,"y":0,"disabled":true},
            {"name":"invalid"}
        ]"#;
        assert_eq!(
            parse_hyprland_monitors_json(json),
            [
                MonitorRect {
                    x: 0,
                    y: 0,
                    width: 1280,
                    height: 720
                },
                MonitorRect {
                    x: -720,
                    y: 0,
                    width: 720,
                    height: 1280
                },
            ]
        );
        assert!(parse_hyprland_monitors_json("invalid").is_empty());
    }

    #[test]
    fn parses_hyprland_cursor_position() {
        assert_eq!(
            parse_hyprland_cursorpos_json(r#"{"x":-21.7,"y":42}"#),
            Some((-22, 42))
        );
        assert_eq!(parse_hyprland_cursorpos_json(r#"{"x":10}"#), None);
        assert_eq!(parse_hyprland_cursorpos_json(r#"{"x":1e10,"y":0}"#), None);
    }

    #[test]
    fn clamps_relative_motion_and_warps_to_monitor_pixels() {
        let mut tracker = VirtualCursorTracker::new(
            vec![
                MonitorRect {
                    x: -100,
                    y: 0,
                    width: 100,
                    height: 100,
                },
                MonitorRect {
                    x: 50,
                    y: 0,
                    width: 100,
                    height: 100,
                },
            ],
            (-50, 0),
        );
        assert_eq!(tracker.apply_relative(60, -10), (-1, 0));
        assert_eq!(tracker.apply_relative(i32::MAX, i32::MAX), (149, 99));
        assert_eq!(tracker.warp(44, 50), (50, 50));
    }

    #[test]
    fn plans_corner_sweep_for_edge_entry() {
        let mut tracker = VirtualCursorTracker::new(Vec::new(), (960, 540));
        assert_eq!(
            tracker.corner_sweep(1915, 500),
            Some([(7680, 0), (0, -4320), (-4, 500)])
        );
        assert_eq!(tracker.warp(1915, 500), (1915, 500));
        assert_eq!(tracker.corner_sweep(960, 540), None);
    }

    #[test]
    fn gives_permission_hint_for_uinput_denial() {
        let status = BackendStatusInfo::new(SessionKind::Wayland, InjectionMethod::Xtest, true);
        let hint = status.hint.unwrap();
        assert!(hint.contains("install-linux.sh") && hint.contains("udev"));
    }
}
