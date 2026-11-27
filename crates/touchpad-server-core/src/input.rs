use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use touchpad_protocol::{HostOs, MouseButton, PrivilegeMode};
use touchpad_storage::{DEFAULT_SCROLL_SENSITIVITY, DEFAULT_SENSITIVITY};

pub const MAX_MOVE_DELTA: f32 = 10_000.0;
pub const MAX_SCROLL_DELTA: f32 = 1_000.0;
pub const MAX_SCROLL_TICKS_PER_EVENT: i32 = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub(crate) const DEFAULT_MONITOR: MonitorRect = MonitorRect {
    x: 0,
    y: 0,
    width: 1920,
    height: 1080,
};

impl MonitorRect {
    pub(crate) fn limits(self) -> Option<[i32; 4]> {
        (self.width > 0 && self.height > 0).then(|| {
            [
                self.x,
                self.y,
                self.x.saturating_add(self.width - 1),
                self.y.saturating_add(self.height - 1),
            ]
        })
    }
}

pub(crate) fn desktop_limits(bounds: &[MonitorRect]) -> [i32; 4] {
    bounds
        .iter()
        .filter_map(|rect| rect.limits())
        .reduce(|a, b| {
            [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ]
        })
        .unwrap_or([0, 0, 1919, 1079])
}

pub trait InputBackend: Send + Sync {
    fn supports_relative_move(&self) -> bool {
        true
    }
    fn move_relative(&self, dx: i32, dy: i32);
    fn move_absolute(&self, x: i32, y: i32);
    fn set_cursor_pos(&self, x: i32, y: i32);
    fn button_press(&self, button: MouseButton);
    fn button_release(&self, button: MouseButton);
    fn scroll(&self, ticks: i32);
    fn cursor_pos(&self) -> Option<(i32, i32)>;
    fn monitor_bounds(&self) -> Vec<MonitorRect>;
    fn is_cursor_locked_for_game(&self) -> bool {
        false
    }
    fn os_kind(&self) -> HostOs;
    fn privilege_mode(&self) -> PrivilegeMode;
}

/// Finds the nearest addressable pixel, including for displays with gaps or negative origins.
pub fn clamp_to_monitors(x: i64, y: i64, monitors: &[MonitorRect]) -> (i32, i32) {
    // The backend only accepts i32 coordinates.
    let x = x.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    let y = y.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    monitors
        .iter()
        .filter_map(|rect| rect.limits())
        .map(|[left, top, right, bottom]| {
            (
                x.clamp(left.into(), right.into()),
                y.clamp(top.into(), bottom.into()),
            )
        })
        .min_by_key(|&(cx, cy)| i128::from(x - cx).pow(2) + i128::from(y - cy).pow(2))
        .map(|(cx, cy)| (cx as i32, cy as i32))
        .unwrap_or_else(|| (x.clamp(0, 1919) as i32, y.clamp(0, 1079) as i32))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAction {
    RelativeMove(i32, i32),
    AbsoluteMove(i32, i32),
    Warp(i32, i32),
    ButtonPress(MouseButton),
    ButtonRelease(MouseButton),
    Scroll(i32),
}

#[derive(Debug, Clone)]
pub struct DryRunConfig {
    pub monitor_bounds: Vec<MonitorRect>,
    pub initial_pos: (i32, i32),
    pub supports_relative_move: bool,
    pub locked_for_game: bool,
    pub os: HostOs,
    pub privilege: PrivilegeMode,
}

impl Default for DryRunConfig {
    fn default() -> Self {
        Self {
            monitor_bounds: vec![DEFAULT_MONITOR],
            initial_pos: (960, 540),
            supports_relative_move: false,
            locked_for_game: false,
            os: if cfg!(windows) {
                HostOs::Windows
            } else {
                HostOs::Linux
            },
            privilege: PrivilegeMode::User,
        }
    }
}

struct DryRunState {
    position: (i32, i32),
    locked: bool,
    actions: Vec<InputAction>,
}

pub struct DryRunBackend {
    config: DryRunConfig,
    state: Mutex<DryRunState>,
}

impl Default for DryRunBackend {
    fn default() -> Self {
        Self::new(DryRunConfig::default())
    }
}

impl DryRunBackend {
    pub fn new(config: DryRunConfig) -> Self {
        Self {
            state: Mutex::new(DryRunState {
                position: config.initial_pos,
                locked: config.locked_for_game,
                actions: Vec::new(),
            }),
            config,
        }
    }

    pub fn actions(&self) -> Vec<InputAction> {
        self.state.lock().unwrap().actions.clone()
    }

    pub fn take_actions(&self) -> Vec<InputAction> {
        std::mem::take(&mut self.state.lock().unwrap().actions)
    }

    pub fn set_locked_for_game(&self, locked: bool) {
        self.state.lock().unwrap().locked = locked;
    }
}

impl InputBackend for DryRunBackend {
    fn supports_relative_move(&self) -> bool {
        self.config.supports_relative_move
    }

    fn move_relative(&self, dx: i32, dy: i32) {
        let mut state = self.state.lock().unwrap();
        state.actions.push(InputAction::RelativeMove(dx, dy));
        if !state.locked {
            state.position = clamp_to_monitors(
                i64::from(state.position.0) + i64::from(dx),
                i64::from(state.position.1) + i64::from(dy),
                &self.config.monitor_bounds,
            );
        }
    }

    fn move_absolute(&self, x: i32, y: i32) {
        let mut state = self.state.lock().unwrap();
        state.actions.push(InputAction::AbsoluteMove(x, y));
        state.position = (x, y);
    }

    fn set_cursor_pos(&self, x: i32, y: i32) {
        let mut state = self.state.lock().unwrap();
        state.actions.push(InputAction::Warp(x, y));
        state.position = (x, y);
    }

    fn button_press(&self, button: MouseButton) {
        self.state
            .lock()
            .unwrap()
            .actions
            .push(InputAction::ButtonPress(button));
    }

    fn button_release(&self, button: MouseButton) {
        self.state
            .lock()
            .unwrap()
            .actions
            .push(InputAction::ButtonRelease(button));
    }

    fn scroll(&self, ticks: i32) {
        self.state
            .lock()
            .unwrap()
            .actions
            .push(InputAction::Scroll(ticks));
    }

    fn cursor_pos(&self) -> Option<(i32, i32)> {
        Some(self.state.lock().unwrap().position)
    }
    fn monitor_bounds(&self) -> Vec<MonitorRect> {
        self.config.monitor_bounds.clone()
    }
    fn is_cursor_locked_for_game(&self) -> bool {
        self.state.lock().unwrap().locked
    }
    fn os_kind(&self) -> HostOs {
        self.config.os
    }
    fn privilege_mode(&self) -> PrivilegeMode {
        self.config.privilege
    }
}

pub struct MouseController {
    backend: Arc<dyn InputBackend>,
    sensitivity: f32,
    scroll_sensitivity: f32,
    remainder_x: f32,
    remainder_y: f32,
    scroll_accumulator: f32,
    last_position: (i32, i32),
    pressed_buttons: HashSet<MouseButton>,
}

impl MouseController {
    pub fn new(backend: Arc<dyn InputBackend>, sensitivity: f32, scroll_sensitivity: f32) -> Self {
        let mut controller = Self {
            last_position: backend.cursor_pos().unwrap_or_default(),
            backend,
            sensitivity: DEFAULT_SENSITIVITY,
            scroll_sensitivity: DEFAULT_SCROLL_SENSITIVITY,
            remainder_x: 0.0,
            remainder_y: 0.0,
            scroll_accumulator: 0.0,
            pressed_buttons: HashSet::new(),
        };
        controller.update_sensitivity(sensitivity, scroll_sensitivity);
        controller
    }

    pub fn update_sensitivity(&mut self, sensitivity: f32, scroll_sensitivity: f32) {
        self.sensitivity = finite_clamp(sensitivity, DEFAULT_SENSITIVITY, 0.1, 10.0);
        self.scroll_sensitivity =
            finite_clamp(scroll_sensitivity, DEFAULT_SCROLL_SENSITIVITY, 0.1, 5.0);
    }

    pub fn move_cursor(&mut self, dx: f32, dy: f32) {
        if !dx.is_finite() || !dy.is_finite() {
            return;
        }
        let sx = accumulate(
            &mut self.remainder_x,
            dx.clamp(-MAX_MOVE_DELTA, MAX_MOVE_DELTA) * self.sensitivity,
        );
        let sy = accumulate(
            &mut self.remainder_y,
            dy.clamp(-MAX_MOVE_DELTA, MAX_MOVE_DELTA) * self.sensitivity,
        );
        if (sx, sy) == (0, 0) {
            return;
        }
        if self.backend.supports_relative_move() {
            self.backend.move_relative(sx, sy);
            self.last_position = (
                self.last_position.0.saturating_add(sx),
                self.last_position.1.saturating_add(sy),
            );
            return;
        }
        let current = self.current_position();
        let x = i64::from(current.0) + i64::from(sx);
        let y = i64::from(current.1) + i64::from(sy);
        let target = clamp_to_monitors(x, y, &self.backend.monitor_bounds());
        if i64::from(target.0) != x {
            self.remainder_x = 0.0;
        }
        if i64::from(target.1) != y {
            self.remainder_y = 0.0;
        }
        self.last_position = target;
        if current != target {
            self.backend.move_absolute(target.0, target.1);
        }
    }

    pub fn warp(&mut self, x: i32, y: i32) {
        self.backend.set_cursor_pos(x, y);
        self.last_position = (x, y);
        self.reset_accumulators();
    }

    pub fn current_position(&self) -> (i32, i32) {
        self.backend.cursor_pos().unwrap_or(self.last_position)
    }

    pub fn remainder_snapshot(&self) -> (f32, f32) {
        (self.remainder_x, self.remainder_y)
    }

    pub fn pressed_buttons(&self) -> &HashSet<MouseButton> {
        &self.pressed_buttons
    }

    pub fn click(&mut self, button: MouseButton) {
        self.set_button_state(button, false);
        self.backend.button_press(button);
        self.backend.button_release(button);
    }

    pub fn set_button_state(&mut self, button: MouseButton, pressed: bool) {
        if pressed {
            if self.pressed_buttons.insert(button) {
                self.backend.button_press(button);
            }
        } else if self.pressed_buttons.remove(&button) {
            self.backend.button_release(button);
        }
    }

    pub fn scroll(&mut self, delta: f32) {
        if !delta.is_finite() {
            return;
        }
        let delta = delta.clamp(-MAX_SCROLL_DELTA, MAX_SCROLL_DELTA) * self.scroll_sensitivity;
        let ticks = accumulate(&mut self.scroll_accumulator, delta);
        let limited = ticks.clamp(-MAX_SCROLL_TICKS_PER_EVENT, MAX_SCROLL_TICKS_PER_EVENT);
        if ticks != limited {
            self.scroll_accumulator = 0.0;
        }
        if limited != 0 {
            self.backend.scroll(limited);
        }
    }

    pub fn release_all_buttons(&mut self) {
        for button in [MouseButton::Left, MouseButton::Right, MouseButton::Middle] {
            self.set_button_state(button, false);
        }
    }

    pub fn reset_accumulators(&mut self) {
        self.remainder_x = 0.0;
        self.remainder_y = 0.0;
        self.scroll_accumulator = 0.0;
    }
}

fn finite_clamp(value: f32, default: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}

fn accumulate(remainder: &mut f32, delta: f32) -> i32 {
    if delta * *remainder < 0.0 {
        *remainder = 0.0;
    }
    let total = *remainder + delta;
    let whole = total as i32;
    *remainder = total - whole as f32;
    whole
}
