use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use touchpad_protocol::{ScreenEdge, DEFAULT_PORT};
use touchpad_storage::{
    android_config_path, default_user_desktop_config_path, migrate_user_config_to_program_data,
    windows_service_config_path, AllowedDevice, AndroidConfig, ConfigStore, DesktopConfig,
    EdgeLink, SavedComputer,
};
use uuid::Uuid;

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(0);

struct TempTestDir {
    path: PathBuf,
}

impl TempTestDir {
    fn new(prefix: &str) -> Self {
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let dir = std::env::temp_dir().join(format!("touchpad_storage_test_{prefix}_{pid}_{seq}"));
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
        std::fs::create_dir_all(&dir).expect("create temp test dir");
        Self { path: dir }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempTestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn desktop_config_round_trip() {
    let temp = TempTestDir::new("desktop_rt");
    let cfg_path = temp.path().join("nested").join("config.json");

    let store = ConfigStore::<DesktopConfig>::new(cfg_path.clone()).expect("open missing store");
    assert_eq!(store.path(), cfg_path.as_path());
    assert!(!cfg_path.exists(), "new() on missing file should not write immediately");

    let desired = DesktopConfig {
        version: 1,
        computer_name: "Workstation-Alpha".to_string(),
        port: 46000,
        sensitivity: 2.25,
        scroll_sensitivity: 1.5,
        require_pairing_approval: false,
        edge_overshoot_px: 40,
        edge_cooldown_ms: 500,
        start_minimized_to_tray: true,
        allowed_devices: vec![
            AllowedDevice {
                client_id: "android-1".to_string(),
                client_name: "Pixel 9".to_string(),
                last_ip: "192.168.1.44".to_string(),
                approved_at_ms: 1_700_000_000_000,
                last_seen_ms: 1_700_000_123_456,
            },
            AllowedDevice {
                client_id: "android-2".to_string(),
                client_name: "Galaxy Tab".to_string(),
                last_ip: "192.168.1.88".to_string(),
                approved_at_ms: 1_700_000_200_000,
                last_seen_ms: 1_700_000_300_000,
            },
        ],
    };

    let saved = store.save(desired.clone()).expect("save desktop config");
    assert_eq!(saved, desired);
    assert_eq!(store.get(), desired);
    assert!(cfg_path.exists());

    // Ensure no stale temp files remain in the target directory.
    let entries: Vec<_> = std::fs::read_dir(cfg_path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec!["config.json".to_string()]);

    let reloaded = ConfigStore::<DesktopConfig>::new(cfg_path).expect("reload desktop config");
    assert_eq!(reloaded.get(), desired);
}

#[test]
fn android_config_round_trip() {
    let temp = TempTestDir::new("android_rt");
    let cfg_path = android_config_path(temp.path());

    let store = ConfigStore::<AndroidConfig>::new(cfg_path.clone()).expect("open missing android store");
    let initial = store.get();
    assert!(Uuid::parse_str(&initial.client_id).is_ok());

    let expected = AndroidConfig {
        version: 1,
        client_id: Uuid::new_v4().to_string(),
        computers: vec![
            SavedComputer {
                id: "pc-left".to_string(),
                name: "Left PC".to_string(),
                ip_address: "10.0.0.10".to_string(),
                port: DEFAULT_PORT,
                connection_method_id: "wifi-lan".to_string(),
                last_os: Some("Windows".to_string()),
                last_privilege: Some("SYSTEM".to_string()),
            },
            SavedComputer {
                id: "pc-right".to_string(),
                name: "Right PC".to_string(),
                ip_address: "10.0.0.20".to_string(),
                port: 45455,
                connection_method_id: "adb-usb".to_string(),
                last_os: Some("Linux".to_string()),
                last_privilege: None,
            },
        ],
        edge_links: vec![EdgeLink {
            from_computer_id: "pc-left".to_string(),
            from_edge: ScreenEdge::Right,
            to_computer_id: "pc-right".to_string(),
            to_edge: ScreenEdge::Left,
        }],
        multi_pc_enabled: true,
        hold_duration_ms: 300,
        haptics_enabled: false,
        default_landscape: false,
        edge_overshoot_px: 36,
        local_sensitivity: 1.4,
    };

    let saved = store.save(expected.clone()).expect("save android config");
    assert_eq!(saved, expected);

    let raw_json = std::fs::read_to_string(&cfg_path).expect("read android json");
    assert!(raw_json.contains("\"RIGHT\""));
    assert!(raw_json.contains("\"LEFT\""));

    let reloaded = ConfigStore::<AndroidConfig>::new(cfg_path).expect("reload android config");
    assert_eq!(reloaded.get(), expected);
}

#[test]
fn forward_compatible_partial_and_unknown_fields() {
    let desktop_json = r#"{
        "port": 47000,
        "future_flag": true,
        "allowed_devices": [
            { "client_id": "c1", "future_nested": [1, 2, 3] }
        ]
    }"#;
    let mut desktop: DesktopConfig =
        serde_json::from_str(desktop_json).expect("deserialize partial desktop json");
    desktop.sanitize();
    assert_eq!(desktop.version, 1);
    assert_eq!(desktop.port, 47000);
    assert!((desktop.sensitivity - 1.6).abs() < f32::EPSILON);
    assert!(desktop.require_pairing_approval);
    assert_eq!(desktop.allowed_devices.len(), 1);
    assert_eq!(desktop.allowed_devices[0].client_id, "c1");

    let android_json = r#"{
        "multi_pc_enabled": true,
        "unknown_v3_field": "hello",
        "computers": [
            { "id": "pc1", "ip_address": "192.168.1.2" }
        ],
        "edge_links": [
            { "from_computer_id": "pc1", "to_computer_id": "missing" }
        ]
    }"#;
    let mut android: AndroidConfig =
        serde_json::from_str(android_json).expect("deserialize partial android json");
    android.sanitize();
    assert_eq!(android.version, 1);
    assert!(Uuid::parse_str(&android.client_id).is_ok());
    assert!(android.multi_pc_enabled);
    assert_eq!(android.computers.len(), 1);
    assert_eq!(android.computers[0].port, DEFAULT_PORT);
    assert!(android.edge_links.is_empty(), "dangling edge link should be pruned");
}

#[test]
fn corrupt_json_recovery_backs_up_and_resets() {
    let temp = TempTestDir::new("corrupt_recovery");
    let cfg_path = temp.path().join("config.json");
    let corrupt_payload = "{ this is not valid json !!!";
    std::fs::write(&cfg_path, corrupt_payload).expect("write corrupt config");

    let store = ConfigStore::<DesktopConfig>::new(cfg_path.clone())
        .expect("corrupt config should recover cleanly");
    assert_eq!(store.get(), DesktopConfig::default());
    assert!(
        !cfg_path.exists(),
        "corrupt original file should have been moved to backup"
    );

    let backups: Vec<PathBuf> = std::fs::read_dir(temp.path())
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name()?.to_string_lossy().into_owned();
            if name.starts_with("config.json.corrupt.") {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(backups.len(), 1, "expected exactly one corrupt backup file");
    assert_eq!(
        std::fs::read_to_string(&backups[0]).unwrap(),
        corrupt_payload
    );

    let updated = store
        .update(|cfg| {
            cfg.port = 45555;
            cfg.sensitivity = 2.0;
        })
        .expect("update after recovery should succeed");
    assert_eq!(updated.port, 45555);
    assert!(cfg_path.exists());

    let reloaded = ConfigStore::<DesktopConfig>::new(cfg_path).expect("reload updated config");
    assert_eq!(reloaded.get(), updated);
}

#[test]
fn desktop_config_sanitization() {
    let mut cfg = DesktopConfig {
        version: 0,
        computer_name: "   ".to_string(),
        port: 0,
        sensitivity: f32::NAN,
        scroll_sensitivity: f32::INFINITY,
        require_pairing_approval: true,
        edge_overshoot_px: 999,
        edge_cooldown_ms: 10,
        start_minimized_to_tray: false,
        allowed_devices: vec![
            AllowedDevice {
                client_id: "   ".to_string(),
                client_name: "No ID or IP".to_string(),
                last_ip: " ".to_string(),
                approved_at_ms: 1,
                last_seen_ms: 1,
            },
            AllowedDevice {
                client_id: " dev-1 ".to_string(),
                client_name: " Old Name ".to_string(),
                last_ip: " 10.0.0.1 ".to_string(),
                approved_at_ms: 100,
                last_seen_ms: 200,
            },
            AllowedDevice {
                client_id: "dev-1".to_string(),
                client_name: "New Name".to_string(),
                last_ip: "10.0.0.99".to_string(),
                approved_at_ms: 100,
                last_seen_ms: 500,
            },
            AllowedDevice {
                client_id: "".to_string(),
                client_name: "IP Only Old".to_string(),
                last_ip: " 10.0.0.5 ".to_string(),
                approved_at_ms: 50,
                last_seen_ms: 100,
            },
            AllowedDevice {
                client_id: "  ".to_string(),
                client_name: "IP Only New".to_string(),
                last_ip: "10.0.0.5".to_string(),
                approved_at_ms: 50,
                last_seen_ms: 300,
            },
        ],
    };

    cfg.sanitize();
    assert_eq!(cfg.version, 1);
    assert!(!cfg.computer_name.is_empty());
    assert_eq!(cfg.port, DEFAULT_PORT);
    assert!((cfg.sensitivity - 1.6).abs() < f32::EPSILON);
    assert!((cfg.scroll_sensitivity - 1.0).abs() < f32::EPSILON);
    assert_eq!(cfg.edge_overshoot_px, 500);
    assert_eq!(cfg.edge_cooldown_ms, 50);

    assert_eq!(cfg.allowed_devices.len(), 2);
    assert_eq!(cfg.allowed_devices[0].client_id, "dev-1");
    assert_eq!(cfg.allowed_devices[0].client_name, "New Name");
    assert_eq!(cfg.allowed_devices[0].last_ip, "10.0.0.99");
    assert_eq!(cfg.allowed_devices[0].last_seen_ms, 500);

    assert_eq!(cfg.allowed_devices[1].client_id, "");
    assert_eq!(cfg.allowed_devices[1].client_name, "IP Only New");
    assert_eq!(cfg.allowed_devices[1].last_ip, "10.0.0.5");

    // Check upper clamp bounds & negative infinity handling.
    cfg.sensitivity = 10.0;
    cfg.scroll_sensitivity = f32::NEG_INFINITY;
    cfg.edge_cooldown_ms = 99_999;
    cfg.sanitize();
    assert!((cfg.sensitivity - 4.0).abs() < f32::EPSILON);
    assert!((cfg.scroll_sensitivity - 1.0).abs() < f32::EPSILON);
    assert_eq!(cfg.edge_cooldown_ms, 5000);
}

#[test]
fn android_config_sanitization() {
    let mut cfg = AndroidConfig {
        version: 0,
        client_id: "   ".to_string(),
        computers: vec![
            SavedComputer {
                id: "first-id".to_string(),
                name: "".to_string(),
                ip_address: " 192.168.1.10 ".to_string(),
                port: 0,
                connection_method_id: "wifi".to_string(),
                last_os: Some("   ".to_string()),
                last_privilege: Some(" user ".to_string()),
            },
            SavedComputer {
                id: "duplicate-id".to_string(),
                name: "Backfilled Name".to_string(),
                ip_address: "192.168.1.10".to_string(),
                port: DEFAULT_PORT,
                connection_method_id: "usb".to_string(),
                last_os: Some("Windows".to_string()),
                last_privilege: Some("Admin".to_string()),
            },
            SavedComputer {
                id: "empty-ip".to_string(),
                name: "Invalid".to_string(),
                ip_address: "   ".to_string(),
                port: 45454,
                connection_method_id: String::new(),
                last_os: None,
                last_privilege: None,
            },
            SavedComputer {
                id: "second-id".to_string(),
                name: "Second PC".to_string(),
                ip_address: "192.168.1.20".to_string(),
                port: 45455,
                connection_method_id: "wifi".to_string(),
                last_os: None,
                last_privilege: None,
            },
        ],
        edge_links: vec![
            EdgeLink {
                from_computer_id: "first-id".to_string(),
                from_edge: ScreenEdge::Right,
                to_computer_id: "second-id".to_string(),
                to_edge: ScreenEdge::Left,
            },
            // Duplicate of first valid link
            EdgeLink {
                from_computer_id: " first-id ".to_string(),
                from_edge: ScreenEdge::Right,
                to_computer_id: " second-id ".to_string(),
                to_edge: ScreenEdge::Left,
            },
            // Dangling link referencing removed duplicate computer id
            EdgeLink {
                from_computer_id: "duplicate-id".to_string(),
                from_edge: ScreenEdge::Top,
                to_computer_id: "second-id".to_string(),
                to_edge: ScreenEdge::Bottom,
            },
            // Self-referential link
            EdgeLink {
                from_computer_id: "first-id".to_string(),
                from_edge: ScreenEdge::Left,
                to_computer_id: "first-id".to_string(),
                to_edge: ScreenEdge::Right,
            },
        ],
        multi_pc_enabled: true,
        hold_duration_ms: 50,
        haptics_enabled: true,
        default_landscape: true,
        edge_overshoot_px: 1200,
        local_sensitivity: f32::NAN,
    };

    cfg.sanitize();
    assert_eq!(cfg.version, 1);
    let parsed_uuid = Uuid::parse_str(&cfg.client_id).expect("valid auto-generated uuid");
    assert_eq!(parsed_uuid.get_version_num(), 4);

    assert_eq!(cfg.hold_duration_ms, 120);
    assert_eq!(cfg.edge_overshoot_px, 500);
    assert!((cfg.local_sensitivity - 1.0).abs() < f32::EPSILON);

    assert_eq!(cfg.computers.len(), 2);
    assert_eq!(cfg.computers[0].id, "first-id");
    assert_eq!(cfg.computers[0].port, DEFAULT_PORT);
    assert_eq!(cfg.computers[0].name, "Backfilled Name");
    assert_eq!(cfg.computers[0].last_os.as_deref(), Some("Windows"));
    assert_eq!(cfg.computers[0].last_privilege.as_deref(), Some("user"));
    assert_eq!(cfg.computers[1].id, "second-id");

    assert_eq!(cfg.edge_links.len(), 1);
    assert_eq!(cfg.edge_links[0].from_computer_id, "first-id");
    assert_eq!(cfg.edge_links[0].to_computer_id, "second-id");

    // Check upper clamp bounds.
    cfg.hold_duration_ms = 9999;
    cfg.local_sensitivity = 9.0;
    cfg.sanitize();
    assert_eq!(cfg.hold_duration_ms, 1500);
    assert!((cfg.local_sensitivity - 3.0).abs() < f32::EPSILON);
}

#[test]
fn migrate_user_config_to_program_data_flows() {
    let temp = TempTestDir::new("migration");
    let user_path = temp.path().join("user").join("config.json");
    let program_data_path = temp.path().join("program_data").join("config.json");

    // 1. Missing user config -> false.
    let changed = migrate_user_config_to_program_data(&user_path, &program_data_path)
        .expect("missing user config should return Ok(false)");
    assert!(!changed);
    assert!(!program_data_path.exists());

    // 2. Missing program_data_path -> copies user config.
    let user_store = ConfigStore::<DesktopConfig>::new(user_path.clone()).unwrap();
    user_store
        .save(DesktopConfig {
            version: 1,
            computer_name: "UserCustomPC".to_string(),
            port: 45460,
            sensitivity: 2.1,
            scroll_sensitivity: 1.0,
            require_pairing_approval: false,
            edge_overshoot_px: 28,
            edge_cooldown_ms: 350,
            start_minimized_to_tray: true,
            allowed_devices: vec![
                AllowedDevice {
                    client_id: "phone-newer-in-pd".to_string(),
                    client_name: "Phone Old".to_string(),
                    last_ip: "192.168.1.10".to_string(),
                    approved_at_ms: 100,
                    last_seen_ms: 200,
                },
                AllowedDevice {
                    client_id: "phone-user-only".to_string(),
                    client_name: "Tablet".to_string(),
                    last_ip: "192.168.1.11".to_string(),
                    approved_at_ms: 150,
                    last_seen_ms: 250,
                },
            ],
        })
        .unwrap();

    let copied = migrate_user_config_to_program_data(&user_path, &program_data_path)
        .expect("initial migration should succeed");
    assert!(copied);
    let initial_pd = ConfigStore::<DesktopConfig>::new(program_data_path.clone())
        .unwrap()
        .get();
    assert_eq!(initial_pd.computer_name, "UserCustomPC");
    assert_eq!(initial_pd.port, 45460);
    assert_eq!(initial_pd.allowed_devices.len(), 2);

    // 3. Existing program_data_path with newer allowed_device + custom setting,
    //    while user_path gets an older update for that device plus a brand-new device.
    let pd_store = ConfigStore::<DesktopConfig>::new(program_data_path.clone()).unwrap();
    pd_store
        .save(DesktopConfig {
            version: 1,
            computer_name: "ServicePC".to_string(),
            port: 45999,
            sensitivity: 1.6, // default in target -> should pick up user's non-default 2.1
            scroll_sensitivity: 1.0,
            require_pairing_approval: true,
            edge_overshoot_px: 28,
            edge_cooldown_ms: 350,
            start_minimized_to_tray: false,
            allowed_devices: vec![
                AllowedDevice {
                    client_id: "phone-newer-in-pd".to_string(),
                    client_name: "Phone Fresh Service".to_string(),
                    last_ip: "192.168.1.99".to_string(),
                    approved_at_ms: 100,
                    last_seen_ms: 9000,
                },
                AllowedDevice {
                    client_id: "phone-updated-by-user".to_string(),
                    client_name: "Stale Service Name".to_string(),
                    last_ip: "192.168.1.12".to_string(),
                    approved_at_ms: 100,
                    last_seen_ms: 300,
                },
            ],
        })
        .unwrap();

    user_store
        .update(|cfg| {
            cfg.allowed_devices.push(AllowedDevice {
                client_id: "phone-updated-by-user".to_string(),
                client_name: "Newer User Name".to_string(),
                last_ip: "192.168.1.120".to_string(),
                approved_at_ms: 100,
                last_seen_ms: 4000,
            });
            cfg.allowed_devices.push(AllowedDevice {
                client_id: "phone-brand-new".to_string(),
                client_name: "Brand New Phone".to_string(),
                last_ip: "192.168.1.13".to_string(),
                approved_at_ms: 500,
                last_seen_ms: 500,
            });
        })
        .unwrap();

    let merged = migrate_user_config_to_program_data(&user_path, &program_data_path)
        .expect("merge migration should succeed");
    assert!(merged);

    let after = ConfigStore::<DesktopConfig>::new(program_data_path.clone())
        .unwrap()
        .get();
    // Non-default target settings were preserved; default sensitivity adopted user's 2.1.
    assert_eq!(after.computer_name, "ServicePC");
    assert_eq!(after.port, 45999);
    assert!((after.sensitivity - 2.1).abs() < f32::EPSILON);

    assert_eq!(after.allowed_devices.len(), 4);
    let newer_pd = after
        .allowed_devices
        .iter()
        .find(|d| d.client_id == "phone-newer-in-pd")
        .unwrap();
    assert_eq!(newer_pd.client_name, "Phone Fresh Service");
    assert_eq!(newer_pd.last_ip, "192.168.1.99");
    assert_eq!(newer_pd.last_seen_ms, 9000);

    let updated_from_user = after
        .allowed_devices
        .iter()
        .find(|d| d.client_id == "phone-updated-by-user")
        .unwrap();
    assert_eq!(updated_from_user.client_name, "Newer User Name");
    assert_eq!(updated_from_user.last_seen_ms, 4000);

    assert!(after
        .allowed_devices
        .iter()
        .any(|d| d.client_id == "phone-user-only"));
    assert!(after
        .allowed_devices
        .iter()
        .any(|d| d.client_id == "phone-brand-new"));

    // 4. Running migration again immediately is a no-op.
    let second_run = migrate_user_config_to_program_data(&user_path, &program_data_path)
        .expect("idempotent second migration");
    assert!(!second_run);
}

#[test]
fn path_helpers_work() {
    let user_path = default_user_desktop_config_path();
    #[cfg(windows)]
    assert!(
        user_path.ends_with(Path::new("TouchPadRemote").join("config.json")),
        "unexpected Windows user config path: {user_path:?}"
    );
    #[cfg(not(windows))]
    assert!(
        user_path.ends_with(Path::new("touchpad-remote").join("config.json")),
        "unexpected Unix user config path: {user_path:?}"
    );

    let service_path = windows_service_config_path();
    assert!(
        service_path.ends_with(Path::new("TouchPadRemote").join("config.json")),
        "unexpected Windows service config path: {service_path:?}"
    );

    let android_path = android_config_path(Path::new("/data/user/0/com.touchpad/files"));
    assert_eq!(
        android_path,
        PathBuf::from("/data/user/0/com.touchpad/files").join("touchpad_config.json")
    );
}

#[test]
fn config_store_thread_safety() {
    let temp = TempTestDir::new("thread_safety");
    let cfg_path = temp.path().join("config.json");
    let store = Arc::new(ConfigStore::<DesktopConfig>::new(cfg_path.clone()).unwrap());

    let thread_count = 8usize;
    let mut handles = Vec::with_capacity(thread_count);
    for idx in 0..thread_count {
        let store_cloned = Arc::clone(&store);
        handles.push(std::thread::spawn(move || {
            store_cloned
                .update(|cfg| {
                    cfg.allowed_devices.push(AllowedDevice {
                        client_id: format!("thread-dev-{idx}"),
                        client_name: format!("Device {idx}"),
                        last_ip: format!("10.0.0.{}", idx + 1),
                        approved_at_ms: 1000 + idx as u64,
                        last_seen_ms: 2000 + idx as u64,
                    });
                })
                .expect("concurrent update should succeed");
        }));
    }

    for handle in handles {
        handle.join().expect("thread should not panic");
    }

    let snapshot = store.get();
    assert_eq!(snapshot.allowed_devices.len(), thread_count);

    let reloaded = ConfigStore::<DesktopConfig>::new(cfg_path).unwrap().get();
    assert_eq!(reloaded.allowed_devices.len(), thread_count);
    for idx in 0..thread_count {
        let id = format!("thread-dev-{idx}");
        assert!(
            reloaded.allowed_devices.iter().any(|d| d.client_id == id),
            "missing device {id}"
        );
    }
}
