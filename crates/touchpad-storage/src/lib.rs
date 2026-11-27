//! Atomic JSON configuration and paired-device storage.

mod error;
mod model;
mod paths;
mod store;

pub use error::StorageError;
pub use model::{
    AllowedDevice, AndroidConfig, DesktopConfig, EdgeLink, Sanitizable, SavedComputer,
    CONFIG_VERSION, DEFAULT_EDGE_COOLDOWN_MS, DEFAULT_EDGE_OVERSHOOT_PX, DEFAULT_HOLD_DURATION_MS,
    DEFAULT_LOCAL_SENSITIVITY, DEFAULT_SCROLL_SENSITIVITY, DEFAULT_SENSITIVITY,
    MAX_EDGE_COOLDOWN_MS, MAX_EDGE_OVERSHOOT_PX, MAX_HOLD_DURATION_MS, MAX_LOCAL_SENSITIVITY,
    MAX_SCROLL_SENSITIVITY, MAX_SENSITIVITY, MIN_EDGE_COOLDOWN_MS, MIN_HOLD_DURATION_MS,
    MIN_LOCAL_SENSITIVITY, MIN_SCROLL_SENSITIVITY, MIN_SENSITIVITY,
};
pub use paths::{
    android_config_path, default_user_desktop_config_path, migrate_user_config_to_program_data,
    windows_service_config_path,
};
pub use store::ConfigStore;
