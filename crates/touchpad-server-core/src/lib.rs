//! Core UDP server, pairing, mouse math, and Multi-PC edge detection.

pub mod edge;
pub mod input;
pub mod network;
pub mod server;

pub use edge::{compute_enter_position, EdgeConfig, EdgeDetector, EdgeHit};
pub use input::{
    clamp_to_monitors, DryRunBackend, DryRunConfig, InputAction, InputBackend, MonitorRect,
    MouseController,
};
pub use network::{get_local_ip_addresses, resolve_best_local_ip_for, NetworkInfo};
pub use server::{
    EventCallback, PairingClientInfo, ServerError, ServerEvent, ServerSnapshot, TouchpadServer,
};
