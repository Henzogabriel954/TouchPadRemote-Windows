use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use touchpad_protocol::{
    ConnectionMethod, HostOs, MouseButton, PairingStatus, PrivilegeMode, ScreenEdge,
    TouchpadPacket as Packet,
};
use touchpad_server_core::*;
use touchpad_storage::{ConfigStore, DesktopConfig};

const LOCAL: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    server: TouchpadServer,
    backend: Arc<DryRunBackend>,
    events: mpsc::Receiver<ServerEvent>,
    path: PathBuf,
}

impl Fixture {
    fn new(require_approval: bool) -> Self {
        Self::with_backend(require_approval, DryRunConfig::default())
    }

    fn with_backend(require_approval: bool, config: DryRunConfig) -> Self {
        let path = std::env::temp_dir().join(format!(
            "touchpad-core-{}-{}.json",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let store = ConfigStore::<DesktopConfig>::new(path.clone()).unwrap();
        store
            .update(|settings| {
                settings.computer_name = "Test PC".into();
                settings.require_pairing_approval = require_approval;
                settings.sensitivity = 1.0;
            })
            .unwrap();
        let backend = Arc::new(DryRunBackend::new(config));
        let (tx, events) = mpsc::channel();
        let mut server = TouchpadServer::with_port(backend.clone(), store, 0).with_event_callback(
            Arc::new(move |event| {
                let _ = tx.send(event);
            }),
        );
        server.start().unwrap();
        Self {
            server,
            backend,
            events,
            path,
        }
    }

    fn client(&self) -> UdpSocket {
        self.client_from(Ipv4Addr::LOCALHOST)
    }

    fn client_from(&self, ip: Ipv4Addr) -> UdpSocket {
        let client = UdpSocket::bind((ip, 0)).unwrap();
        client.connect((LOCAL, self.server.actual_port())).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client
    }

    fn approve(&self, socket: &UdpSocket, id: &str) {
        assert_eq!(pair(socket, id), PairingStatus::Pending);
        self.server
            .approve_pairing(socket.local_addr().unwrap().ip())
            .unwrap();
        assert_eq!(pair(socket, id), PairingStatus::Approved);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.stop();
        let _ = std::fs::remove_file(&self.path);
    }
}

fn send(socket: &UdpSocket, packet: Packet) {
    socket.send(&packet.to_bytes()).unwrap();
}

fn recv(socket: &UdpSocket) -> Packet {
    let mut buf = [0; 2048];
    let len = socket.recv(&mut buf).unwrap();
    Packet::from_bytes(&buf[..len]).unwrap()
}

fn host_info(socket: &UdpSocket) -> Packet {
    let packet = recv(socket);
    assert!(matches!(packet, Packet::HostInfo { .. }));
    packet
}

fn pair(socket: &UdpSocket, id: &str) -> PairingStatus {
    send(
        socket,
        Packet::PairRequest {
            client_id: id.into(),
            client_name: format!("Phone {id}"),
        },
    );
    let Packet::PairResponse {
        status,
        server_name,
    } = recv(socket)
    else {
        panic!("expected pairing response")
    };
    assert_eq!(server_name, "Test PC");
    host_info(socket);
    status
}

fn barrier(socket: &UdpSocket) {
    send(socket, Packet::Ping { timestamp: 42 });
    // Unsolicited host metadata can precede a reply when a client sends controls first.
    loop {
        match recv(socket) {
            Packet::Pong { timestamp: 42, .. } => break,
            Packet::HostInfo { .. } => {}
            packet => panic!("unexpected packet {packet:?}"),
        }
    }
    host_info(socket);
}

fn rect(x: i32, y: i32, width: i32, height: i32) -> MonitorRect {
    MonitorRect {
        x,
        y,
        width,
        height,
    }
}

#[test]
fn discover_returns_name_lan_address_and_bound_port() {
    let fixture = Fixture::new(false);
    let socket = fixture.client();
    send(
        &socket,
        Packet::DiscoverRequest {
            client_name: "Phone".into(),
        },
    );
    assert_eq!(
        recv(&socket),
        Packet::DiscoverResponse {
            computer_name: "Test PC".into(),
            ip_address: LOCAL.to_string(),
            port: fixture.server.actual_port(),
            method: ConnectionMethod::NetworkLan,
        }
    );
    assert!(matches!(
        fixture.events.recv().unwrap(),
        ServerEvent::StateChanged { running: true, .. }
    ));
    assert!(
        matches!(fixture.events.recv_timeout(Duration::from_secs(2)).unwrap(), ServerEvent::DiscoveryRequested { client_name, .. } if client_name == "Phone")
    );
}

#[test]
fn ping_echoes_timestamp_and_sends_virtual_desktop_info() {
    let fixture = Fixture::with_backend(
        false,
        DryRunConfig {
            monitor_bounds: vec![rect(-200, -50, 200, 150), rect(0, 0, 1920, 1080)],
            os: HostOs::Windows,
            privilege: PrivilegeMode::System,
            ..DryRunConfig::default()
        },
    );
    let socket = fixture.client();
    send(&socket, Packet::Ping { timestamp: -9 });
    assert_eq!(
        recv(&socket),
        Packet::Pong {
            timestamp: -9,
            server_name: "Test PC".into()
        }
    );
    let expected = Packet::HostInfo {
        os: HostOs::Windows,
        width: 2120,
        height: 1130,
        privilege: PrivilegeMode::System,
    };
    assert_eq!(host_info(&socket), expected);
    send(&socket, expected.clone());
    assert_eq!(host_info(&socket), expected);
}

#[test]
fn move_and_click_update_count_events_and_backend() {
    let fixture = Fixture::new(false);
    let socket = fixture.client();
    send(&socket, Packet::Move { dx: 2.0, dy: -3.0 });
    send(
        &socket,
        Packet::Click {
            button: MouseButton::Left,
        },
    );
    barrier(&socket);
    assert_eq!(fixture.server.snapshot().packet_count, 2);
    assert_eq!(fixture.server.snapshot().last_client_ip, Some(LOCAL));
    assert_eq!(
        fixture.backend.actions(),
        [
            InputAction::AbsoluteMove(962, 537),
            InputAction::ButtonPress(MouseButton::Left),
            InputAction::ButtonRelease(MouseButton::Left),
        ]
    );
    let counts: Vec<_> = fixture
        .events
        .try_iter()
        .filter_map(|event| match event {
            ServerEvent::PacketReceived { total_packets, .. } => Some(total_packets),
            _ => None,
        })
        .collect();
    assert_eq!(counts, [1, 2]);
}

#[test]
fn subpixel_movement_clamps_to_nearest_monitor_and_scroll_accumulates() {
    let backend = Arc::new(DryRunBackend::new(DryRunConfig {
        monitor_bounds: vec![rect(-100, 0, 100, 100), rect(100, 0, 100, 100)],
        initial_pos: (-2, 50),
        ..DryRunConfig::default()
    }));
    let mut mouse = MouseController::new(backend.clone(), 1.0, 1.0);
    mouse.move_cursor(0.5, 0.0);
    assert!(backend.actions().is_empty());
    mouse.move_cursor(0.5, 0.0);
    assert_eq!(mouse.current_position(), (-1, 50));
    mouse.move_cursor(100.0, 0.0);
    assert_eq!(mouse.current_position(), (100, 50));
    mouse.move_cursor(900.0, 0.0);
    assert_eq!(mouse.current_position(), (199, 50));
    mouse.scroll(0.5);
    mouse.scroll(0.5);
    assert_eq!(backend.actions().last(), Some(&InputAction::Scroll(1)));
}

#[test]
fn stop_closes_socket_and_notifies_listener() {
    let mut fixture = Fixture::new(false);
    let port = fixture.server.actual_port();
    fixture.server.stop();
    assert!(!fixture.server.is_running());
    assert!(UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port)).is_ok());
    assert!(fixture.events.try_iter().any(
        |event| matches!(event, ServerEvent::StateChanged { running: false, port: p } if p == port)
    ));
}

#[test]
fn local_ips_are_valid_non_loopback_ipv4() {
    for ip in get_local_ip_addresses() {
        let ip: Ipv4Addr = ip.parse().unwrap();
        assert!(
            !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() && !ip.is_multicast()
        );
    }
}

#[test]
fn pairing_blocks_controls_until_approved() {
    let fixture = Fixture::new(true);
    let socket = fixture.client();
    let movement = Packet::Move { dx: 3.0, dy: 0.0 };
    send(&socket, movement.clone());
    barrier(&socket);
    assert_eq!(fixture.server.snapshot().packet_count, 0);
    assert_eq!(pair(&socket, "alpha"), PairingStatus::Pending);
    send(&socket, movement.clone());
    barrier(&socket);
    assert!(fixture.backend.actions().is_empty());
    assert!(fixture.events.try_iter().any(|event| matches!(event, ServerEvent::PairingRequested(client) if client.client_id == "alpha")));
    fixture.server.approve_pairing(LOCAL).unwrap();
    send(&socket, movement);
    barrier(&socket);
    assert_eq!(fixture.server.snapshot().packet_count, 1);
    assert_eq!(pair(&socket, "alpha"), PairingStatus::Approved);
}

#[test]
fn relative_game_mode_emits_unclamped_deltas() {
    let backend = Arc::new(DryRunBackend::new(DryRunConfig {
        supports_relative_move: true,
        locked_for_game: true,
        initial_pos: (1919, 540),
        ..DryRunConfig::default()
    }));
    let mut mouse = MouseController::new(backend.clone(), 2.0, 1.0);
    mouse.move_cursor(1500.0, -1500.0);
    assert_eq!(backend.actions(), [InputAction::RelativeMove(3000, -3000)]);
    assert_eq!(mouse.current_position(), (1919, 540));
}

#[test]
fn extreme_floats_do_not_poison_accumulators() {
    let backend = Arc::new(DryRunBackend::new(DryRunConfig {
        supports_relative_move: true,
        ..DryRunConfig::default()
    }));
    let mut mouse = MouseController::new(backend.clone(), 10.0, 5.0);
    mouse.move_cursor(f32::MAX, -f32::MAX);
    mouse.scroll(f32::MAX);
    mouse.scroll(0.1);
    mouse.move_cursor(-0.11, 0.11);
    assert_eq!(
        backend.actions(),
        [
            InputAction::RelativeMove(100_000, -100_000),
            InputAction::Scroll(12),
            InputAction::RelativeMove(-1, 1),
        ]
    );
}

#[test]
fn nan_defaults_and_direction_reversal_reset_remainders() {
    let backend = Arc::new(DryRunBackend::default());
    let mut mouse = MouseController::new(backend.clone(), f32::NAN, f32::INFINITY);
    mouse.move_cursor(0.5, 0.0);
    assert_eq!(mouse.remainder_snapshot(), (0.8, 0.0));
    mouse.move_cursor(f32::NAN, 100.0);
    mouse.move_cursor(-0.625, 0.0);
    mouse.scroll(0.9);
    mouse.scroll(f32::INFINITY);
    mouse.scroll(-1.0);
    assert_eq!(
        backend.actions(),
        [InputAction::AbsoluteMove(959, 540), InputAction::Scroll(-1)]
    );
}

#[test]
fn revoke_releases_held_buttons_and_rejects_during_cooldown() {
    let fixture = Fixture::new(true);
    let socket = fixture.client();
    fixture.approve(&socket, "alpha");
    send(
        &socket,
        Packet::ButtonState {
            button: MouseButton::Left,
            pressed: true,
        },
    );
    barrier(&socket);
    fixture.server.revoke_device("alpha").unwrap();
    assert_eq!(pair(&socket, "alpha"), PairingStatus::Rejected);
    send(
        &socket,
        Packet::Click {
            button: MouseButton::Left,
        },
    );
    barrier(&socket);
    assert_eq!(
        fixture.backend.actions(),
        [
            InputAction::ButtonPress(MouseButton::Left),
            InputAction::ButtonRelease(MouseButton::Left)
        ]
    );
    assert!(fixture.server.snapshot().config.allowed_devices.is_empty());
}

#[test]
fn client_id_migrates_and_conflicting_client_evicts_ip() {
    let fixture = Fixture::new(true);
    let original = fixture.client();
    let migrated = fixture.client_from(Ipv4Addr::new(127, 0, 0, 2));
    fixture.approve(&original, "alpha");
    assert_eq!(pair(&migrated, "alpha"), PairingStatus::Approved);
    assert_eq!(
        fixture.server.snapshot().config.allowed_devices[0].last_ip,
        "127.0.0.2"
    );
    send(
        &original,
        Packet::Click {
            button: MouseButton::Left,
        },
    );
    barrier(&original);
    assert!(fixture.backend.actions().is_empty());
    send(
        &migrated,
        Packet::ButtonState {
            button: MouseButton::Left,
            pressed: true,
        },
    );
    barrier(&migrated);
    assert_eq!(pair(&migrated, "beta"), PairingStatus::Pending);
    assert!(fixture.server.snapshot().approved_clients.is_empty());
    assert_eq!(
        fixture.server.snapshot().pending_pairings[0].client_id,
        "beta"
    );
    assert_eq!(
        fixture.backend.actions().last(),
        Some(&InputAction::ButtonRelease(MouseButton::Left))
    );
    assert_eq!(pair(&original, "alpha"), PairingStatus::Approved);
}

#[test]
fn loopback_resolves_to_loopback() {
    assert_eq!(
        resolve_best_local_ip_for(LOCAL).as_deref(),
        Some("127.0.0.1")
    );
}

#[test]
fn allowlist_and_last_seen_survive_restart_and_revocation() {
    let mut fixture = Fixture::new(true);
    fixture.approve(&fixture.client(), "alpha");
    fixture.server.stop();
    let store = ConfigStore::<DesktopConfig>::new(fixture.path.clone()).unwrap();
    let previous = store
        .update(|settings| settings.allowed_devices[0].last_seen_ms = 2)
        .unwrap();
    fixture.server = TouchpadServer::with_port(fixture.backend.clone(), store, 0);
    fixture.server.start().unwrap();
    let socket = fixture.client();
    send(&socket, Packet::Move { dx: 1.0, dy: 0.0 });
    barrier(&socket);
    assert_eq!(fixture.server.snapshot().packet_count, 1);
    let disk = ConfigStore::<DesktopConfig>::new(fixture.path.clone())
        .unwrap()
        .get();
    assert!(disk.allowed_devices[0].last_seen_ms > previous.allowed_devices[0].last_seen_ms);
    assert_eq!(pair(&socket, "alpha"), PairingStatus::Approved);
    fixture.server.revoke_device("alpha").unwrap();
    fixture.server.stop();
    let store = ConfigStore::<DesktopConfig>::new(fixture.path.clone()).unwrap();
    assert!(store.get().allowed_devices.is_empty());
    fixture.server = TouchpadServer::with_port(fixture.backend.clone(), store, 0);
    fixture.server.start().unwrap();
    assert_eq!(pair(&fixture.client(), "alpha"), PairingStatus::Pending);
}

#[test]
fn edge_detector_ignores_internal_edges_lock_and_buttons_then_accumulates_and_cools_down() {
    let backend = DryRunBackend::new(DryRunConfig {
        monitor_bounds: vec![rect(-100, 0, 100, 101), rect(0, 0, 100, 101)],
        initial_pos: (-1, 50),
        ..DryRunConfig::default()
    });
    let mut detector = EdgeDetector::new(
        EdgeConfig {
            right: true,
            ..EdgeConfig::default()
        },
        350,
    );
    let now = Instant::now();
    assert_eq!(detector.evaluate_at(&backend, 40.0, 0.0, false, now), None);
    backend.set_cursor_pos(99, 50);
    backend.set_locked_for_game(true);
    assert_eq!(detector.evaluate_at(&backend, 40.0, 0.0, false, now), None);
    backend.set_locked_for_game(false);
    assert_eq!(detector.evaluate_at(&backend, 40.0, 0.0, true, now), None);
    assert_eq!(detector.evaluate_at(&backend, 15.0, 0.0, false, now), None);
    assert_eq!(detector.evaluate_at(&backend, -1.0, 0.0, false, now), None);
    assert_eq!(detector.evaluate_at(&backend, 14.0, 0.0, false, now), None);
    assert_eq!(
        detector.evaluate_at(&backend, 14.0, 0.0, false, now),
        Some(EdgeHit {
            edge: ScreenEdge::Right,
            ratio: 0.5
        })
    );
    assert_eq!(
        detector.evaluate_at(&backend, 99.0, 0.0, false, now + Duration::from_millis(349)),
        None
    );
    assert!(detector
        .evaluate_at(&backend, 99.0, 0.0, false, now + Duration::from_millis(350))
        .is_some());
    assert_eq!(
        compute_enter_position(ScreenEdge::Left, 0.5, &backend.monitor_bounds()),
        (-96, 50)
    );
    assert_eq!(
        compute_enter_position(
            ScreenEdge::Top,
            0.5,
            &[rect(-50, 0, 50, 10), rect(0, 20, 50, 10)]
        ),
        (-1, 4)
    );
}

#[test]
fn cursor_enter_warps_and_suppresses_edge_hit_then_udp_move_triggers_it() {
    let fixture = Fixture::with_backend(
        true,
        DryRunConfig {
            monitor_bounds: vec![rect(0, 0, 101, 101)],
            initial_pos: (50, 50),
            ..DryRunConfig::default()
        },
    );
    let socket = fixture.client();
    send(
        &socket,
        Packet::CursorEnter {
            edge: ScreenEdge::Right,
            ratio: 0.25,
        },
    );
    barrier(&socket);
    assert!(fixture.backend.actions().is_empty());
    fixture.approve(&socket, "alpha");
    send(
        &socket,
        Packet::EdgeConfig {
            left: false,
            right: true,
            top: false,
            bottom: false,
            overshoot_px: 12,
        },
    );
    send(
        &socket,
        Packet::CursorEnter {
            edge: ScreenEdge::Right,
            ratio: 0.25,
        },
    );
    send(&socket, Packet::Move { dx: 20.0, dy: 0.0 });
    barrier(&socket);
    assert_eq!(
        fixture.backend.actions(),
        [
            InputAction::Warp(96, 25),
            InputAction::AbsoluteMove(100, 25)
        ]
    );
    fixture.server.update_settings(1.0, 1.0, 0, true).unwrap();
    let cooldown = fixture.server.snapshot().config.edge_cooldown_ms;
    std::thread::sleep(Duration::from_millis(cooldown));
    send(&socket, Packet::Move { dx: 12.0, dy: 0.0 });
    assert_eq!(
        recv(&socket),
        Packet::EdgeHit {
            edge: ScreenEdge::Right,
            ratio: 0.25
        }
    );
}

#[test]
fn cursor_leave_releases_drag_and_resets_move_and_scroll_accumulators() {
    let fixture = Fixture::new(true);
    let socket = fixture.client();
    fixture.approve(&socket, "alpha");
    send(
        &socket,
        Packet::ButtonState {
            button: MouseButton::Right,
            pressed: true,
        },
    );
    send(&socket, Packet::Move { dx: 0.75, dy: 0.0 });
    send(&socket, Packet::Scroll { delta_y: 0.75 });
    send(
        &socket,
        Packet::CursorLeave {
            edge: ScreenEdge::Left,
        },
    );
    send(&socket, Packet::Move { dx: 0.75, dy: 0.0 });
    send(&socket, Packet::Scroll { delta_y: 0.75 });
    barrier(&socket);
    assert_eq!(
        fixture.backend.actions(),
        [
            InputAction::ButtonPress(MouseButton::Right),
            InputAction::ButtonRelease(MouseButton::Right)
        ]
    );
}

#[test]
fn settings_persist_and_apply_live() {
    let fixture = Fixture::new(true);
    let socket = fixture.client();
    fixture
        .server
        .update_settings(2.0, 3.0, 400, false)
        .unwrap();
    send(&socket, Packet::Move { dx: 2.0, dy: 0.0 });
    send(&socket, Packet::Scroll { delta_y: 1.0 });
    barrier(&socket);
    assert_eq!(
        fixture.backend.actions(),
        [InputAction::AbsoluteMove(964, 540), InputAction::Scroll(3)]
    );
    let disk = ConfigStore::<DesktopConfig>::new(fixture.path.clone())
        .unwrap()
        .get();
    assert_eq!(disk, fixture.server.snapshot().config);
    assert_eq!(disk.edge_cooldown_ms, 400);
    assert!(!disk.require_pairing_approval);
}
