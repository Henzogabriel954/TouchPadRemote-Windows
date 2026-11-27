use super::{
    parse_hyprland_cursorpos_json, parse_hyprland_monitors_json, BackendStatusInfo,
    InjectionMethod, MonitorRect, SessionKind, VirtualCursorTracker,
};
use evdev::{
    uinput::{VirtualDevice, VirtualDeviceBuilder},
    AttributeSet, BusType, EventType, InputEvent, InputId, Key, RelativeAxisType,
};
use std::{
    env,
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::Mutex,
    time::Duration,
};
use touchpad_protocol::{HostOs, MouseButton, PrivilegeMode};
use touchpad_server_core::{clamp_to_monitors, input::MAX_SCROLL_TICKS_PER_EVENT, InputBackend};
use x11rb::{
    connection::Connection,
    protocol::{
        randr::ConnectionExt as _,
        xproto::{self, ConnectionExt as _, Window},
        xtest::ConnectionExt as _,
    },
    rust_connection::RustConnection,
    CURRENT_TIME, NONE,
};

#[derive(Debug, thiserror::Error)]
#[error("Linux input injection unavailable: {source} {}", status.hint.unwrap_or_default())]
pub struct LinuxInputError {
    pub status: BackendStatusInfo,
    #[source]
    source: io::Error,
}

pub struct LinuxInputBackend {
    status: BackendStatusInfo,
    uinput: Option<Mutex<VirtualDevice>>,
    x11: Option<X11>,
    hyprland: Option<Hyprland>,
    cursor: Mutex<VirtualCursorTracker>,
}

impl LinuxInputBackend {
    pub fn new() -> Result<Self, LinuxInputError> {
        let display = env::var_os("DISPLAY").filter(|value| !value.is_empty());
        let hyprland = Hyprland::from_env();
        let session = match env::var("XDG_SESSION_TYPE").as_deref() {
            Ok("wayland") => SessionKind::Wayland,
            Ok("x11") if display.is_some() => SessionKind::X11,
            _ if env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty())
                || env::var_os("WAYLAND_SOCKET").is_some()
                || hyprland.is_some() =>
            {
                SessionKind::Wayland
            }
            _ if display.is_some() => SessionKind::X11,
            _ => SessionKind::Headless,
        };

        let (uinput, x11, injection, denied) = match create_uinput() {
            Ok(device) => (
                Some(Mutex::new(device)),
                (session == SessionKind::X11).then(X11::connect).flatten(),
                InjectionMethod::Uinput,
                false,
            ),
            Err(source) => {
                let denied = source.kind() == io::ErrorKind::PermissionDenied;
                let can_fallback =
                    display.is_some() && (denied || source.kind() == io::ErrorKind::NotFound);
                let x11 = can_fallback
                    .then(X11::connect)
                    .flatten()
                    .filter(X11::has_xtest);
                if x11.is_none() {
                    return Err(LinuxInputError {
                        status: BackendStatusInfo::new(
                            session,
                            InjectionMethod::Unavailable,
                            denied,
                        ),
                        source,
                    });
                }
                (None, x11, InjectionMethod::Xtest, denied)
            }
        };
        let monitors = match session {
            SessionKind::Wayland => hyprland.as_ref().map(Hyprland::monitors),
            SessionKind::X11 => x11.as_ref().map(X11::monitors),
            SessionKind::Headless => None,
        }
        .unwrap_or_default();
        let initial = match session {
            SessionKind::Wayland => hyprland.as_ref().and_then(Hyprland::cursor_pos),
            SessionKind::X11 => x11.as_ref().and_then(X11::cursor_pos),
            SessionKind::Headless => None,
        }
        .unwrap_or_else(|| {
            let monitor = monitors.first().copied().unwrap_or(super::DEFAULT_MONITOR);
            (
                monitor.x.saturating_add(monitor.width / 2),
                monitor.y.saturating_add(monitor.height / 2),
            )
        });
        Ok(Self {
            status: BackendStatusInfo::new(session, injection, denied),
            uinput,
            x11,
            hyprland,
            cursor: Mutex::new(VirtualCursorTracker::new(monitors, initial)),
        })
    }

    pub fn status(&self) -> &BackendStatusInfo {
        &self.status
    }

    fn inject_relative(&self, dx: i32, dy: i32) -> bool {
        if let Some(device) = &self.uinput {
            emit(
                device,
                &[
                    InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_X.0, dx),
                    InputEvent::new(EventType::RELATIVE, RelativeAxisType::REL_Y.0, dy),
                ],
            )
        } else {
            self.x11
                .as_ref()
                .is_some_and(|x11| x11.fake_input(xproto::MOTION_NOTIFY_EVENT, 1, dx, dy))
        }
    }

    fn set_button(&self, button: MouseButton, pressed: bool) {
        let (key, x_button) = match button {
            MouseButton::Left => (Key::BTN_LEFT, 1),
            MouseButton::Right => (Key::BTN_RIGHT, 3),
            MouseButton::Middle => (Key::BTN_MIDDLE, 2),
        };
        if let Some(device) = &self.uinput {
            emit(
                device,
                &[InputEvent::new(EventType::KEY, key.0, i32::from(pressed))],
            );
        } else if let Some(x11) = &self.x11 {
            x11.button(x_button, pressed);
        }
    }
}

impl InputBackend for LinuxInputBackend {
    fn move_relative(&self, dx: i32, dy: i32) {
        let mut tracker = self.cursor.lock().unwrap();
        if self.inject_relative(dx, dy) {
            tracker.apply_relative(dx, dy);
        }
    }

    fn move_absolute(&self, x: i32, y: i32) {
        self.set_cursor_pos(x, y);
    }

    fn set_cursor_pos(&self, x: i32, y: i32) {
        let monitors = self.monitor_bounds();
        let (x, y) = clamp_to_monitors(x.into(), y.into(), &monitors);
        let mut tracker = self.cursor.lock().unwrap();
        let warped = match self.status.session {
            SessionKind::X11 => self.x11.as_ref().is_some_and(|x11| x11.warp(x, y)),
            SessionKind::Wayland => self.hyprland.as_ref().is_some_and(|hypr| hypr.warp(x, y)),
            SessionKind::Headless => false,
        };
        if warped {
            *tracker = VirtualCursorTracker::new(monitors, (x, y));
            return;
        }
        if self.status.session == SessionKind::Wayland && self.uinput.is_some() {
            if let Some(moves) = tracker.corner_sweep(x, y) {
                for (dx, dy) in moves {
                    if !self.inject_relative(dx, dy) {
                        return;
                    }
                    tracker.apply_relative(dx, dy);
                }
                tracker.warp(x, y);
                return;
            }
        }
        let (old_x, old_y) = tracker.position();
        if self.inject_relative(x.saturating_sub(old_x), y.saturating_sub(old_y)) {
            tracker.warp(x, y);
        }
    }

    fn button_press(&self, button: MouseButton) {
        self.set_button(button, true);
    }

    fn button_release(&self, button: MouseButton) {
        self.set_button(button, false);
    }

    fn scroll(&self, ticks: i32) {
        if ticks == 0 {
            return;
        }
        if let Some(device) = &self.uinput {
            emit(
                device,
                &[InputEvent::new(
                    EventType::RELATIVE,
                    RelativeAxisType::REL_WHEEL.0,
                    ticks.saturating_neg(),
                )],
            );
        } else if let Some(x11) = &self.x11 {
            let button = if ticks > 0 { 5 } else { 4 };
            for _ in 0..ticks.unsigned_abs().min(MAX_SCROLL_TICKS_PER_EVENT as u32) {
                if !x11.button(button, true) || !x11.button(button, false) {
                    break;
                }
            }
        }
    }

    fn cursor_pos(&self) -> Option<(i32, i32)> {
        let queried = match self.status.session {
            SessionKind::X11 => self.x11.as_ref().and_then(X11::cursor_pos),
            SessionKind::Wayland => self.hyprland.as_ref().and_then(Hyprland::cursor_pos),
            SessionKind::Headless => None,
        };
        queried.or_else(|| Some(self.cursor.lock().unwrap().position()))
    }

    fn monitor_bounds(&self) -> Vec<MonitorRect> {
        let queried = match self.status.session {
            SessionKind::X11 => self.x11.as_ref().map(X11::monitors),
            SessionKind::Wayland => self.hyprland.as_ref().map(Hyprland::monitors),
            SessionKind::Headless => None,
        };
        queried
            .filter(|monitors| !monitors.is_empty())
            .unwrap_or_else(|| self.cursor.lock().unwrap().monitor_bounds().to_vec())
    }

    fn os_kind(&self) -> HostOs {
        HostOs::Linux
    }

    fn privilege_mode(&self) -> PrivilegeMode {
        if self.uinput.is_some() {
            PrivilegeMode::Uinput
        } else {
            PrivilegeMode::Xtest
        }
    }
}

fn create_uinput() -> io::Result<VirtualDevice> {
    let keys: AttributeSet<Key> = [Key::BTN_LEFT, Key::BTN_RIGHT, Key::BTN_MIDDLE]
        .into_iter()
        .collect();
    let axes: AttributeSet<RelativeAxisType> = [
        RelativeAxisType::REL_X,
        RelativeAxisType::REL_Y,
        RelativeAxisType::REL_WHEEL,
    ]
    .into_iter()
    .collect();

    VirtualDeviceBuilder::new()?
        .name("TouchPad Remote Virtual Mouse")
        .input_id(InputId::new(BusType::BUS_USB, 0, 0, 1))
        .with_keys(&keys)?
        .with_relative_axes(&axes)?
        .build()
}

fn emit(device: &Mutex<VirtualDevice>, events: &[InputEvent]) -> bool {
    // evdev appends SYN_REPORT to each emitted frame.
    match device.lock().unwrap().emit(events) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(%error, "Failed to emit uinput events");
            false
        }
    }
}

struct X11 {
    connection: RustConnection,
    screen: usize,
}

impl X11 {
    fn connect() -> Option<Self> {
        let (connection, screen) = x11rb::connect(None).ok()?;
        Some(Self { connection, screen })
    }

    fn root(&self) -> Window {
        self.connection.setup().roots[self.screen].root
    }

    fn has_xtest(&self) -> bool {
        self.connection
            .xtest_get_version(2, 2)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some()
    }

    fn monitors(&self) -> Vec<MonitorRect> {
        let root = self.root();
        let mut monitors = Vec::new();
        if let Some(resources) = self
            .connection
            .randr_get_screen_resources_current(root)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
        {
            for crtc in resources.crtcs {
                let info = self
                    .connection
                    .randr_get_crtc_info(crtc, resources.config_timestamp)
                    .ok()
                    .and_then(|cookie| cookie.reply().ok());
                if let Some(info) = info.filter(|info| {
                    info.status == x11rb::protocol::randr::SetConfig::SUCCESS
                        && info.mode != NONE
                        && info.width > 0
                        && info.height > 0
                }) {
                    monitors.push(MonitorRect {
                        x: info.x.into(),
                        y: info.y.into(),
                        width: info.width.into(),
                        height: info.height.into(),
                    });
                }
            }
        }
        if monitors.is_empty() {
            if let Some(root) = self
                .connection
                .get_geometry(root)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
            {
                monitors.push(MonitorRect {
                    x: 0,
                    y: 0,
                    width: root.width.into(),
                    height: root.height.into(),
                });
            }
        }
        monitors
    }

    fn cursor_pos(&self) -> Option<(i32, i32)> {
        let pointer = self
            .connection
            .query_pointer(self.root())
            .ok()?
            .reply()
            .ok()?;
        pointer
            .same_screen
            .then_some((pointer.root_x.into(), pointer.root_y.into()))
    }

    fn warp(&self, x: i32, y: i32) -> bool {
        self.connection
            .warp_pointer(NONE, self.root(), 0, 0, 0, 0, x_coord(x), x_coord(y))
            .is_ok_and(|cookie| cookie.check().is_ok())
    }

    fn fake_input(&self, event: u8, detail: u8, x: i32, y: i32) -> bool {
        self.connection
            .xtest_fake_input(event, detail, CURRENT_TIME, NONE, x_coord(x), x_coord(y), 0)
            .is_ok_and(|cookie| cookie.check().is_ok())
    }

    fn button(&self, button: u8, pressed: bool) -> bool {
        let event = if pressed {
            xproto::BUTTON_PRESS_EVENT
        } else {
            xproto::BUTTON_RELEASE_EVENT
        };
        self.fake_input(event, button, 0, 0)
    }
}

fn x_coord(value: i32) -> i16 {
    value.clamp(i16::MIN.into(), i16::MAX.into()) as i16
}

struct Hyprland {
    socket: PathBuf,
}

impl Hyprland {
    fn from_env() -> Option<Self> {
        let runtime = PathBuf::from(env::var_os("XDG_RUNTIME_DIR")?);
        let signature =
            env::var_os("HYPRLAND_INSTANCE_SIGNATURE").filter(|value| !value.is_empty())?;
        Some(Self {
            socket: runtime.join("hypr").join(signature).join(".socket.sock"),
        })
    }

    fn request(&self, command: &str) -> io::Result<String> {
        let mut socket = UnixStream::connect(&self.socket)?;
        let timeout = Some(Duration::from_millis(150));
        socket.set_read_timeout(timeout)?;
        socket.set_write_timeout(timeout)?;
        socket.write_all(command.as_bytes())?;
        let mut reply = String::new();
        socket.take(1024 * 1024).read_to_string(&mut reply)?;
        Ok(reply)
    }

    fn monitors(&self) -> Vec<MonitorRect> {
        self.request("j/monitors")
            .map_or_else(|_| Vec::new(), |reply| parse_hyprland_monitors_json(&reply))
    }

    fn cursor_pos(&self) -> Option<(i32, i32)> {
        parse_hyprland_cursorpos_json(&self.request("j/cursorpos").ok()?)
    }

    fn warp(&self, x: i32, y: i32) -> bool {
        self.request(&format!("/dispatch movecursor {x} {y}"))
            .is_ok_and(|reply| reply.trim() == "ok")
    }
}
