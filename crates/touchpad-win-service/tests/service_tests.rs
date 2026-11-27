use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use touchpad_protocol::{HostOs, MouseButton, PrivilegeMode};
use touchpad_server_core::{
    DryRunBackend, DryRunConfig, InputAction, InputBackend, MonitorRect, PairingClientInfo,
    ServerSnapshot,
};
use touchpad_storage::{AllowedDevice, ConfigStore, DesktopConfig};
use touchpad_win_service::{
    decode_json_line, encode_json_line, ControlRequest, ControlResponse, InputCmd, InputReply,
    CONTROL_PIPE_NAME, INPUT_PIPE_NAME, MAX_IPC_LINE_BYTES, SERVICE_NAME,
};

static TEST_PIPE_SEQ: AtomicU64 = AtomicU64::new(1);

#[cfg(windows)]
fn unique_pipe_name(prefix: &str) -> String {
    let pid = std::process::id();
    let seq = TEST_PIPE_SEQ.fetch_add(1, Ordering::Relaxed);
    format!(r"\\.\pipe\TouchPadRemoteTest_{prefix}_{pid}_{seq}")
}

fn temp_config_path(prefix: &str) -> PathBuf {
    let pid = std::process::id();
    let seq = TEST_PIPE_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("touchpad_win_svc_test_{prefix}_{pid}_{seq}.json"))
}

#[test]
fn public_constants_match_contract() {
    assert_eq!(SERVICE_NAME, "TouchPadRemoteService");
    assert_eq!(CONTROL_PIPE_NAME, r"\\.\pipe\TouchPadRemoteControl");
    assert_eq!(INPUT_PIPE_NAME, r"\\.\pipe\TouchPadRemoteSysInput");
}

#[test]
fn control_request_and_response_json_round_trip() {
    let mut cfg = DesktopConfig::default();
    cfg.computer_name = "Workstation-01".to_string();
    cfg.port = 46464;
    cfg.sensitivity = 2.5;
    cfg.scroll_sensitivity = 1.75;
    cfg.require_pairing_approval = false;
    cfg.edge_overshoot_px = 42;
    cfg.edge_cooldown_ms = 600;
    cfg.start_minimized_to_tray = true;

    let requests = vec![
        ControlRequest::GetSnapshot,
        ControlRequest::ApprovePairing {
            ip_or_id: "192.168.1.50".to_string(),
        },
        ControlRequest::RejectPairing {
            ip_or_id: "client-abc".to_string(),
        },
        ControlRequest::RevokeDevice {
            client_id: "client-123".to_string(),
            ip_address: "192.168.1.99".to_string(),
        },
        ControlRequest::UpdateConfig(cfg.clone()),
        ControlRequest::StartServer,
        ControlRequest::StopServer,
    ];

    for req in requests {
        let encoded = encode_json_line(&req).expect("encode ControlRequest");
        assert!(encoded.ends_with(b"\n"));
        assert!(!encoded[..encoded.len() - 1].contains(&b'\n'));
        let decoded: ControlRequest = decode_json_line(&encoded).expect("decode ControlRequest");
        assert_eq!(decoded, req);
    }

    let snapshot = ServerSnapshot {
        running: true,
        actual_port: 46464,
        packet_count: 1234,
        last_client_ip: Some("192.168.1.50".parse::<IpAddr>().unwrap()),
        pending_pairings: vec![PairingClientInfo {
            client_id: "pending-1".to_string(),
            client_name: "Pixel".to_string(),
            ip_address: "192.168.1.51".parse().unwrap(),
            requested_at_ms: 100,
        }],
        approved_clients: vec![PairingClientInfo {
            client_id: "approved-1".to_string(),
            client_name: "Galaxy".to_string(),
            ip_address: "192.168.1.50".parse().unwrap(),
            requested_at_ms: 90,
        }],
        config: cfg,
    };

    let responses = vec![
        ControlResponse::Ok,
        ControlResponse::Snapshot(snapshot.clone()),
        ControlResponse::Error("some error".to_string()),
    ];

    for resp in responses {
        let mut encoded = encode_json_line(&resp).expect("encode ControlResponse");
        // Verify CRLF trimming works identically to LF.
        encoded.pop();
        encoded.extend_from_slice(b"\r\n");
        let decoded: ControlResponse = decode_json_line(&encoded).expect("decode ControlResponse");
        assert_eq!(decoded, resp);
    }

    assert_eq!(
        ControlResponse::Snapshot(snapshot.clone())
            .into_snapshot()
            .unwrap(),
        snapshot
    );
    assert!(ControlResponse::Ok.into_result().is_ok());
    assert!(ControlResponse::Error("boom".into()).into_result().is_err());

    assert!(decode_json_line::<ControlRequest>(b"\n").is_err());
    assert!(decode_json_line::<ControlRequest>(b"not-json\n").is_err());
    let oversized = vec![b' '; MAX_IPC_LINE_BYTES + 1];
    assert!(decode_json_line::<ControlRequest>(&oversized).is_err());
}

#[test]
fn input_cmd_and_reply_json_round_trip() {
    let cmds = vec![
        InputCmd::MoveRelative { dx: -15, dy: 24 },
        InputCmd::SetCursorPos { x: -300, y: 800 },
        InputCmd::ButtonPress {
            button: MouseButton::Left,
        },
        InputCmd::ButtonRelease {
            button: MouseButton::Middle,
        },
        InputCmd::Scroll { ticks: -3 },
        InputCmd::CursorPos,
        InputCmd::MonitorBounds,
        InputCmd::IsLockedForGame,
    ];

    for cmd in cmds {
        let encoded = encode_json_line(&cmd).expect("encode InputCmd");
        let decoded: InputCmd = decode_json_line(&encoded).expect("decode InputCmd");
        assert_eq!(decoded, cmd);
    }

    let replies = vec![
        InputReply::Ack,
        InputReply::CursorPos(Some((120, -45))),
        InputReply::CursorPos(None),
        InputReply::MonitorBounds(vec![
            MonitorRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            MonitorRect {
                x: -1280,
                y: 0,
                width: 1280,
                height: 720,
            },
        ]),
        InputReply::LockedForGame(true),
        InputReply::Error("desktop switch denied".to_string()),
    ];

    for reply in replies {
        let encoded = encode_json_line(&reply).expect("encode InputReply");
        let decoded: InputReply = decode_json_line(&encoded).expect("decode InputReply");
        assert_eq!(decoded, reply);
    }
}

#[cfg(windows)]
#[test]
fn sddl_strings_validate_and_system_only_pipe_denies_non_system_client() {
    use touchpad_win_service::pipe::{connect_pipe_client, create_named_pipe_server};
    use touchpad_win_service::{validate_sddl, CONTROL_PIPE_SDDL, SYSTEM_ONLY_SDDL};

    assert_eq!(SYSTEM_ONLY_SDDL, "D:(A;;GA;;;SY)");
    assert_eq!(CONTROL_PIPE_SDDL, "D:(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x12008b;;;IU)");

    validate_sddl(SYSTEM_ONLY_SDDL).expect("SYSTEM_ONLY_SDDL must parse cleanly");
    validate_sddl(CONTROL_PIPE_SDDL).expect("CONTROL_PIPE_SDDL must parse cleanly");
    validate_sddl(touchpad_win_service::pipe::SYSTEM_INPUT_PIPE_SDDL)
        .expect("SYSTEM-owned input pipe descriptor must parse cleanly");
    assert!(validate_sddl("INVALID_SDDL_STRING").is_err());

    if !touchpad_input_win::is_running_as_system() {
        let owner_pipe_name = unique_pipe_name("pipe_owner");
        let _owner_pipe = create_named_pipe_server(&owner_pipe_name, None, 1, true)
            .expect("create ordinary local pipe");
        let client = connect_pipe_client(&owner_pipe_name, 500)
            .expect("connect to ordinary local pipe");
        assert_eq!(
            client.require_system_owner().unwrap_err(),
            "Pipe server is not owned by LocalSystem"
        );

        let pipe_name = unique_pipe_name("sddl_acl");
        if let Ok(_server_pipe) =
            create_named_pipe_server(&pipe_name, Some(SYSTEM_ONLY_SDDL), 1, true)
        {
            let client_err = connect_pipe_client(&pipe_name, 500)
                .err()
                .expect("Non-SYSTEM client must be rejected by D:(A;;GA;;;SY)");
            assert!(
                client_err.contains("ACCESS_DENIED") || client_err.contains("5"),
                "Expected ACCESS_DENIED, got: {client_err}"
            );
        }
    }
}

#[cfg(windows)]
#[test]
fn pipe_input_backend_round_trip_and_reconnect_with_mock_agent() {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::thread;

    use touchpad_win_service::pipe::connect_pipe_client;
    use touchpad_win_service::InputPipeServer;

    let pipe_name = unique_pipe_name("input_rt");
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (backend, server) =
        InputPipeServer::start_with_timeout(pipe_name.clone(), None, Arc::clone(&stop_flag), 1_000)
            .expect("start InputPipeServer");

    // Host capability must always report Windows + PrivilegeMode::System.
    assert_eq!(backend.os_kind(), HostOs::Windows);
    assert_eq!(backend.privilege_mode(), PrivilegeMode::System);
    assert!(backend.supports_relative_move());
    assert!(!backend.is_connected());

    let monitors = vec![
        MonitorRect {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        },
        MonitorRect {
            x: -1600,
            y: 0,
            width: 1600,
            height: 900,
        },
    ];
    let dry_run = Arc::new(DryRunBackend::new(DryRunConfig {
        monitor_bounds: monitors.clone(),
        initial_pos: (500, 300),
        supports_relative_move: true,
        locked_for_game: false,
        os: HostOs::Windows,
        privilege: PrivilegeMode::User,
    }));

    let spawn_mock_agent = |pipe_name: String, dry_run: Arc<DryRunBackend>| {
        thread::spawn(move || {
            let mut client = connect_pipe_client(&pipe_name, 2_000).expect("connect mock agent");
            while let Ok(cmd) = client.read_json_line::<InputCmd>(2_000, None) {
                let reply = match cmd {
                    InputCmd::MoveRelative { dx, dy } => {
                        dry_run.move_relative(dx, dy);
                        InputReply::Ack
                    }
                    InputCmd::SetCursorPos { x, y } => {
                        dry_run.set_cursor_pos(x, y);
                        InputReply::Ack
                    }
                    InputCmd::ButtonPress { button } => {
                        dry_run.button_press(button);
                        InputReply::Ack
                    }
                    InputCmd::ButtonRelease { button } => {
                        dry_run.button_release(button);
                        InputReply::Ack
                    }
                    InputCmd::Scroll { ticks } => {
                        dry_run.scroll(ticks);
                        InputReply::Ack
                    }
                    InputCmd::CursorPos => InputReply::CursorPos(dry_run.cursor_pos()),
                    InputCmd::MonitorBounds => InputReply::MonitorBounds(dry_run.monitor_bounds()),
                    InputCmd::IsLockedForGame => {
                        InputReply::LockedForGame(dry_run.is_cursor_locked_for_game())
                    }
                };
                if client.write_json_line(&reply, 1_000).is_err() {
                    break;
                }
            }
        })
    };

    let agent1 = spawn_mock_agent(pipe_name.clone(), Arc::clone(&dry_run));

    // Wait briefly for the acceptor thread to attach the first mock agent pipe.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !backend.is_connected() && std::time::Instant::now() < deadline {
        thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(backend.is_connected());

    backend.move_relative(25, -10);
    assert_eq!(backend.cursor_pos(), Some((525, 290)));
    backend.move_absolute(-200, 450);
    assert_eq!(backend.cursor_pos(), Some((-200, 450)));
    backend.set_cursor_pos(100, 200);
    assert_eq!(backend.cursor_pos(), Some((100, 200)));
    backend.button_press(MouseButton::Right);
    backend.button_release(MouseButton::Right);
    backend.scroll(-2);
    assert_eq!(backend.monitor_bounds(), monitors);
    assert!(!backend.is_cursor_locked_for_game());
    dry_run.set_locked_for_game(true);
    assert!(backend.is_cursor_locked_for_game());
    dry_run.set_locked_for_game(false);
    assert_eq!(backend.privilege_mode(), PrivilegeMode::System);

    assert_eq!(
        dry_run.take_actions(),
        vec![
            InputAction::RelativeMove(25, -10),
            InputAction::Warp(-200, 450),
            InputAction::Warp(100, 200),
            InputAction::ButtonPress(MouseButton::Right),
            InputAction::ButtonRelease(MouseButton::Right),
            InputAction::Scroll(-2),
        ]
    );

    // Simulate desktop agent exit + new session agent reconnect.
    backend.detach_pipe();
    agent1.join().expect("join first mock agent");

    let agent2 = spawn_mock_agent(pipe_name, Arc::clone(&dry_run));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while !backend.is_connected() && std::time::Instant::now() < deadline {
        thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(backend.is_connected());

    backend.move_relative(5, 5);
    assert_eq!(backend.cursor_pos(), Some((105, 205)));

    server.stop_and_join();
    agent2.join().expect("join second mock agent");
}

#[cfg(windows)]
#[test]
fn control_pipe_server_end_to_end_rpc() {
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex};

    use touchpad_win_service::ipc_client::query_control_on_pipe;
    use touchpad_win_service::{ControlPipeServer, ManagedServer};

    let cfg_path = temp_config_path("ctrl_rpc");
    let mut initial_cfg = DesktopConfig::default();
    initial_cfg.allowed_devices.push(AllowedDevice {
        client_id: "phone-1".to_string(),
        client_name: "Pixel".to_string(),
        last_ip: "192.168.1.40".to_string(),
        approved_at_ms: 1000,
        last_seen_ms: 2000,
    });
    let store = Arc::new(ConfigStore::new(cfg_path.clone()).expect("open temp store"));
    store.save(initial_cfg).expect("seed temp store");

    let backend = Arc::new(DryRunBackend::new(DryRunConfig {
        privilege: PrivilegeMode::System,
        ..DryRunConfig::default()
    }));
    let managed = Arc::new(Mutex::new(ManagedServer::with_port_override(
        backend,
        Arc::clone(&store),
        0,
    )));
    let stop_flag = Arc::new(AtomicBool::new(false));
    let pipe_name = unique_pipe_name("ctrl");

    let ctrl_server =
        ControlPipeServer::start(pipe_name.clone(), None, Arc::clone(&managed), stop_flag)
            .expect("start ControlPipeServer");

    let snap = query_control_on_pipe(&pipe_name, &ControlRequest::GetSnapshot)
        .expect("GetSnapshot")
        .into_snapshot()
        .unwrap();
    assert!(!snap.running);
    assert_eq!(snap.approved_clients.len(), 1);

    query_control_on_pipe(&pipe_name, &ControlRequest::StartServer)
        .expect("StartServer")
        .into_result()
        .unwrap();

    let snap = query_control_on_pipe(&pipe_name, &ControlRequest::GetSnapshot)
        .expect("GetSnapshot running")
        .into_snapshot()
        .unwrap();
    assert!(snap.running);
    assert_ne!(snap.actual_port, 0);

    let mut next_cfg = DesktopConfig::default();
    next_cfg.computer_name = "Studio-PC".to_string();
    next_cfg.port = 45999;
    next_cfg.sensitivity = 2.75;
    next_cfg.scroll_sensitivity = 1.8;
    next_cfg.require_pairing_approval = false;
    next_cfg.edge_overshoot_px = 45;
    next_cfg.edge_cooldown_ms = 550;
    next_cfg.start_minimized_to_tray = true;
    // Empty allowed_devices in request must NOT wipe persisted paired devices.
    next_cfg.allowed_devices.clear();

    query_control_on_pipe(&pipe_name, &ControlRequest::UpdateConfig(next_cfg))
        .expect("UpdateConfig")
        .into_result()
        .unwrap();

    let snap = query_control_on_pipe(&pipe_name, &ControlRequest::GetSnapshot)
        .expect("GetSnapshot updated")
        .into_snapshot()
        .unwrap();
    assert_eq!(snap.config.computer_name, "Studio-PC");
    assert_eq!(snap.config.port, 45999);
    assert!((snap.config.sensitivity - 2.75).abs() < 0.001);
    assert!((snap.config.scroll_sensitivity - 1.8).abs() < 0.001);
    assert!(!snap.config.require_pairing_approval);
    assert_eq!(snap.config.edge_overshoot_px, 45);
    assert_eq!(snap.config.edge_cooldown_ms, 550);
    assert!(snap.config.start_minimized_to_tray);
    assert_eq!(snap.config.allowed_devices.len(), 1);

    query_control_on_pipe(
        &pipe_name,
        &ControlRequest::ApprovePairing {
            ip_or_id: "192.168.1.77".to_string(),
        },
    )
    .expect("ApprovePairing")
    .into_result()
    .unwrap();

    let snap = query_control_on_pipe(&pipe_name, &ControlRequest::GetSnapshot)
        .expect("GetSnapshot after approve")
        .into_snapshot()
        .unwrap();
    assert!(snap
        .approved_clients
        .iter()
        .any(|c| c.ip_address.to_string() == "192.168.1.77"));

    query_control_on_pipe(
        &pipe_name,
        &ControlRequest::RejectPairing {
            ip_or_id: "192.168.1.77".to_string(),
        },
    )
    .expect("RejectPairing")
    .into_result()
    .unwrap();

    query_control_on_pipe(
        &pipe_name,
        &ControlRequest::RevokeDevice {
            client_id: "phone-1".to_string(),
            ip_address: "192.168.1.40".to_string(),
        },
    )
    .expect("RevokeDevice")
    .into_result()
    .unwrap();

    let snap = query_control_on_pipe(&pipe_name, &ControlRequest::GetSnapshot)
        .expect("GetSnapshot after revoke")
        .into_snapshot()
        .unwrap();
    assert!(snap.approved_clients.is_empty());
    assert!(snap.config.allowed_devices.is_empty());

    query_control_on_pipe(&pipe_name, &ControlRequest::StopServer)
        .expect("StopServer")
        .into_result()
        .unwrap();

    let snap = query_control_on_pipe(&pipe_name, &ControlRequest::GetSnapshot)
        .expect("GetSnapshot stopped")
        .into_snapshot()
        .unwrap();
    assert!(!snap.running);

    ctrl_server.stop_and_join();
    managed.lock().unwrap().stop();
    let _ = std::fs::remove_file(&cfg_path);
}

#[test]
fn ipc_client_elevation_validation_and_scm_smoke() {
    use std::path::Path;
    use touchpad_win_service::ipc_client::{
        install_service_elevated, is_service_installed, is_service_running,
        uninstall_service_elevated,
    };

    let installed = is_service_installed();
    let running = is_service_running();
    if !installed {
        assert!(!running);
    }

    assert!(install_service_elevated(Path::new("")).is_err());
    assert!(uninstall_service_elevated(Path::new("bad\0path.exe")).is_err());
}
