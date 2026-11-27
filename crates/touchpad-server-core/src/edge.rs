use crate::input::{desktop_limits, InputBackend, MonitorRect, DEFAULT_MONITOR, MAX_MOVE_DELTA};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use touchpad_protocol::ScreenEdge;
use touchpad_storage::{DEFAULT_EDGE_COOLDOWN_MS, DEFAULT_EDGE_OVERSHOOT_PX};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeConfig {
    pub left: bool,
    pub right: bool,
    pub top: bool,
    pub bottom: bool,
    pub overshoot_px: u16,
}

impl Default for EdgeConfig {
    fn default() -> Self {
        Self {
            left: false,
            right: false,
            top: false,
            bottom: false,
            overshoot_px: DEFAULT_EDGE_OVERSHOOT_PX,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EdgeHit {
    pub edge: ScreenEdge,
    pub ratio: f32,
}

pub struct EdgeDetector {
    config: EdgeConfig,
    cooldown: Duration,
    last_switch: Option<Instant>,
    overshoot: Option<(ScreenEdge, f32)>,
}

impl Default for EdgeDetector {
    fn default() -> Self {
        Self::new(EdgeConfig::default(), DEFAULT_EDGE_COOLDOWN_MS)
    }
}

impl EdgeDetector {
    pub fn new(config: EdgeConfig, cooldown_ms: u64) -> Self {
        Self {
            config,
            cooldown: Duration::from_millis(cooldown_ms),
            last_switch: None,
            overshoot: None,
        }
    }

    pub fn update_config(&mut self, config: EdgeConfig) {
        self.config = config;
        self.reset();
    }

    pub fn config(&self) -> EdgeConfig {
        self.config
    }

    pub fn set_cooldown_ms(&mut self, cooldown_ms: u64) {
        self.cooldown = Duration::from_millis(cooldown_ms);
    }

    pub fn reset(&mut self) {
        self.overshoot = None;
    }

    pub fn on_cursor_enter(&mut self) {
        self.begin_cooldown(Instant::now());
    }

    fn begin_cooldown(&mut self, now: Instant) {
        self.last_switch = Some(now);
        self.reset();
    }

    pub fn evaluate_move(
        &mut self,
        backend: &dyn InputBackend,
        dx: f32,
        dy: f32,
        buttons_held: bool,
    ) -> Option<EdgeHit> {
        self.evaluate_at(backend, dx, dy, buttons_held, Instant::now())
    }

    pub fn evaluate_at(
        &mut self,
        backend: &dyn InputBackend,
        dx: f32,
        dy: f32,
        buttons_held: bool,
        now: Instant,
    ) -> Option<EdgeHit> {
        if buttons_held
            || backend.is_cursor_locked_for_game()
            || self
                .last_switch
                .is_some_and(|last| now.saturating_duration_since(last) < self.cooldown)
            || !dx.is_finite()
            || !dy.is_finite()
        {
            self.reset();
            return None;
        }
        let Some((x, y)) = backend.cursor_pos() else {
            self.reset();
            return None;
        };
        let [left, top, right, bottom] = desktop_limits(&backend.monitor_bounds());
        let candidates = [
            (
                ScreenEdge::Left,
                self.config.left && x <= left.saturating_add(1),
                -dx,
                y,
                top,
                bottom,
            ),
            (
                ScreenEdge::Right,
                self.config.right && x >= right.saturating_sub(1),
                dx,
                y,
                top,
                bottom,
            ),
            (
                ScreenEdge::Top,
                self.config.top && y <= top.saturating_add(1),
                -dy,
                x,
                left,
                right,
            ),
            (
                ScreenEdge::Bottom,
                self.config.bottom && y >= bottom.saturating_sub(1),
                dy,
                x,
                left,
                right,
            ),
        ];
        let Some((edge, _, outward, position, min, max)) = candidates
            .into_iter()
            .filter(|(_, pinned, outward, ..)| *pinned && *outward > 0.0)
            .max_by(|a, b| a.2.total_cmp(&b.2))
        else {
            self.reset();
            return None;
        };
        let previous = self
            .overshoot
            .filter(|(previous_edge, _)| *previous_edge == edge)
            .map_or(0.0, |(_, total)| total);
        let total = previous + outward.min(MAX_MOVE_DELTA);
        self.overshoot = Some((edge, total));
        if total < f32::from(self.config.overshoot_px.max(1)) {
            return None;
        }
        self.begin_cooldown(now);
        let span = i64::from(max) - i64::from(min);
        let ratio = ((i64::from(position) - i64::from(min)) as f64 / span.max(1) as f64)
            .clamp(0.0, 1.0) as f32;
        Some(EdgeHit { edge, ratio })
    }

    pub fn compute_enter_position(
        edge: ScreenEdge,
        ratio: f32,
        bounds: &[MonitorRect],
    ) -> (i32, i32) {
        compute_enter_position(edge, ratio, bounds)
    }
}

/// Uses a display that actually touches the requested perimeter, even when its perpendicular
/// coordinate lies in a gap in the virtual desktop.
pub fn compute_enter_position(edge: ScreenEdge, ratio: f32, bounds: &[MonitorRect]) -> (i32, i32) {
    let [left, top, right, bottom] = desktop_limits(bounds);
    let vertical = matches!(edge, ScreenEdge::Left | ScreenEdge::Right);
    let (min, max) = if vertical {
        (top, bottom)
    } else {
        (left, right)
    };
    let ratio = if ratio.is_finite() {
        ratio.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let target =
        (f64::from(min) + f64::from(ratio) * (f64::from(max) - f64::from(min))).round() as i32;
    let position = bounds
        .iter()
        .chain(
            std::iter::once(&DEFAULT_MONITOR)
                .filter(|_| !bounds.iter().any(|rect| rect.limits().is_some())),
        )
        .filter_map(|rect| rect.limits())
        .filter_map(|[l, t, r, b]| {
            let (x, y) = match edge {
                ScreenEdge::Left if l == left => (l.saturating_add(4).min(r), target.clamp(t, b)),
                ScreenEdge::Right if r == right => (r.saturating_sub(4).max(l), target.clamp(t, b)),
                ScreenEdge::Top if t == top => (target.clamp(l, r), t.saturating_add(4).min(b)),
                ScreenEdge::Bottom if b == bottom => {
                    (target.clamp(l, r), b.saturating_sub(4).max(t))
                }
                _ => return None,
            };
            Some((x, y))
        })
        .min_by_key(|&(x, y)| (i64::from(if vertical { y } else { x }) - i64::from(target)).abs());
    position.unwrap_or((left, top))
}
