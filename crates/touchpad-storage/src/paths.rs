use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use directories::BaseDirs;

use crate::error::StorageError;
use crate::model::{device_key, merge_allowed_device, DesktopConfig, DeviceKey};
use crate::store::{atomic_write_json, backup_corrupt_file};

/// Returns the default per-user desktop configuration path:
/// - Windows: `%APPDATA%\TouchPadRemote\config.json`
/// - Linux/Unix: `$XDG_CONFIG_HOME/touchpad-remote/config.json` (or `~/.config/touchpad-remote/config.json`)
pub fn default_user_desktop_config_path() -> PathBuf {
    #[cfg(windows)]
    {
        let base = non_empty_env_path("APPDATA")
            .or_else(|| BaseDirs::new().map(|d| d.config_dir().to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("TouchPadRemote").join("config.json")
    }

    #[cfg(not(windows))]
    {
        let base = non_empty_env_path("XDG_CONFIG_HOME")
            .or_else(|| BaseDirs::new().map(|d| d.config_dir().to_path_buf()))
            .unwrap_or_else(|| PathBuf::from(".config"));
        base.join("touchpad-remote").join("config.json")
    }
}

/// Returns the Windows LocalSystem service configuration path:
/// `C:\ProgramData\TouchPadRemote\config.json` (or `%ALLUSERSPROFILE%\TouchPadRemote\config.json`).
pub fn windows_service_config_path() -> PathBuf {
    let base = non_empty_env_path("ALLUSERSPROFILE")
        .or_else(|| non_empty_env_path("ProgramData"))
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    base.join("TouchPadRemote").join("config.json")
}

/// Returns the Android configuration file path inside the app's `files_dir`.
pub fn android_config_path(files_dir: &Path) -> PathBuf {
    files_dir.join("touchpad_config.json")
}

/// Migrates and merges `allowed_devices` and non-default settings from `user_path` into
/// `program_data_path` without overwriting newer `allowed_devices` or explicit target settings.
///
/// Returns `Ok(true)` if `program_data_path` was created or updated, or `Ok(false)` if no
/// migration changes were needed.
pub fn migrate_user_config_to_program_data(
    user_path: &Path,
    program_data_path: &Path,
) -> Result<bool, StorageError> {
    if user_path == program_data_path {
        return Ok(false);
    }

    let user_raw = match std::fs::read_to_string(user_path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(false),
        Err(source) => {
            return Err(StorageError::Io {
                path: user_path.to_path_buf(),
                source,
            });
        }
    };

    let mut user_cfg: DesktopConfig = serde_json::from_str(&user_raw)?;
    user_cfg.sanitize();

    let mut target_cfg = match std::fs::read_to_string(program_data_path) {
        Ok(raw) => match serde_json::from_str::<DesktopConfig>(&raw) {
            Ok(mut cfg) => {
                cfg.sanitize();
                cfg
            }
            Err(_) => {
                backup_corrupt_file(program_data_path)?;
                atomic_write_json(program_data_path, &user_cfg)?;
                return Ok(true);
            }
        },
        Err(err) if err.kind() == ErrorKind::NotFound => {
            atomic_write_json(program_data_path, &user_cfg)?;
            return Ok(true);
        }
        Err(source) => {
            return Err(StorageError::Io {
                path: program_data_path.to_path_buf(),
                source,
            });
        }
    };

    let original = target_cfg.clone();
    let defaults = DesktopConfig::default();

    merge_settings_from_user(&mut target_cfg, &user_cfg, &defaults);
    merge_allowed_devices(&mut target_cfg, user_cfg.allowed_devices);
    target_cfg.sanitize();

    if target_cfg != original {
        atomic_write_json(program_data_path, &target_cfg)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

fn merge_settings_from_user(
    target: &mut DesktopConfig,
    user: &DesktopConfig,
    defaults: &DesktopConfig,
) {
    if target.computer_name == defaults.computer_name && user.computer_name != defaults.computer_name
    {
        target.computer_name = user.computer_name.clone();
    }
    if target.port == defaults.port && user.port != defaults.port {
        target.port = user.port;
    }
    if f32_eq(target.sensitivity, defaults.sensitivity)
        && !f32_eq(user.sensitivity, defaults.sensitivity)
    {
        target.sensitivity = user.sensitivity;
    }
    if f32_eq(target.scroll_sensitivity, defaults.scroll_sensitivity)
        && !f32_eq(user.scroll_sensitivity, defaults.scroll_sensitivity)
    {
        target.scroll_sensitivity = user.scroll_sensitivity;
    }
    if target.require_pairing_approval == defaults.require_pairing_approval
        && user.require_pairing_approval != defaults.require_pairing_approval
    {
        target.require_pairing_approval = user.require_pairing_approval;
    }
    if target.edge_overshoot_px == defaults.edge_overshoot_px
        && user.edge_overshoot_px != defaults.edge_overshoot_px
    {
        target.edge_overshoot_px = user.edge_overshoot_px;
    }
    if target.edge_cooldown_ms == defaults.edge_cooldown_ms
        && user.edge_cooldown_ms != defaults.edge_cooldown_ms
    {
        target.edge_cooldown_ms = user.edge_cooldown_ms;
    }
    if target.start_minimized_to_tray == defaults.start_minimized_to_tray
        && user.start_minimized_to_tray != defaults.start_minimized_to_tray
    {
        target.start_minimized_to_tray = user.start_minimized_to_tray;
    }
}

fn merge_allowed_devices(
    target: &mut DesktopConfig,
    incoming_devices: Vec<crate::model::AllowedDevice>,
) {
    let mut index_by_key: HashMap<DeviceKey, usize> =
        HashMap::with_capacity(target.allowed_devices.len());
    for (idx, device) in target.allowed_devices.iter().enumerate() {
        if let Some(key) = device_key(device) {
            index_by_key.insert(key, idx);
        }
    }

    for incoming in incoming_devices {
        let Some(key) = device_key(&incoming) else {
            continue;
        };
        if let Some(&idx) = index_by_key.get(&key) {
            merge_allowed_device(&mut target.allowed_devices[idx], incoming);
        } else {
            index_by_key.insert(key, target.allowed_devices.len());
            target.allowed_devices.push(incoming);
        }
    }
}

fn f32_eq(a: f32, b: f32) -> bool {
    (a - b).abs() < f32::EPSILON
}

fn non_empty_env_path(key: &str) -> Option<PathBuf> {
    std::env::var_os(key).and_then(|val| {
        let path = PathBuf::from(val);
        if path.as_os_str().is_empty() {
            None
        } else {
            Some(path)
        }
    })
}
