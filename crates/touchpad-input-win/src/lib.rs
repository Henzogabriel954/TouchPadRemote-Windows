//! Windows mouse input, monitor bounds, game cursor-lock detection, and UAC helpers.

use touchpad_server_core::MonitorRect;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{
    is_process_elevated, is_running_as_system, relaunch_as_admin, WindowsInputBackend,
};

const DEFAULT_MONITOR: MonitorRect = MonitorRect {
    x: 0,
    y: 0,
    width: 1920,
    height: 1080,
};

/// Detect a confined cursor using the full virtual desktop, including negative origins.
pub fn is_clip_rect_locked_for_game(clip: MonitorRect, monitors: &[MonitorRect]) -> bool {
    if clip.width < 0 || clip.height < 0 {
        return false;
    }
    if clip.width <= 16 || clip.height <= 16 {
        return true;
    }

    let (width, height) = virtual_desktop_size(monitors);
    i64::from(clip.width) < width / 2 && i64::from(clip.height) < height / 2
}

fn virtual_desktop_size(monitors: &[MonitorRect]) -> (i64, i64) {
    let bounds = monitors
        .iter()
        .filter(|monitor| monitor.width > 0 && monitor.height > 0)
        .map(|monitor| {
            let left = i64::from(monitor.x);
            let top = i64::from(monitor.y);
            (
                left,
                top,
                left + i64::from(monitor.width),
                top + i64::from(monitor.height),
            )
        })
        .reduce(|a, b| (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3)));

    bounds
        .map(|(left, top, right, bottom)| (right - left, bottom - top))
        .unwrap_or((
            i64::from(DEFAULT_MONITOR.width),
            i64::from(DEFAULT_MONITOR.height),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, width: i32, height: i32) -> MonitorRect {
        MonitorRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn desktop_bounds_include_monitors_above_and_left_and_the_gaps_between_them() {
        let monitors = [
            DEFAULT_MONITOR,
            rect(-1600, 0, 1280, 720),
            rect(0, -900, 1600, 900),
        ];
        assert_eq!(virtual_desktop_size(&monitors), (3520, 1980));
        assert!(is_clip_rect_locked_for_game(
            rect(-1400, -800, 1700, 950),
            &monitors
        ));
        assert!(!is_clip_rect_locked_for_game(
            rect(-1600, -900, 3520, 1980),
            &monitors
        ));
    }

    #[test]
    fn fallback_dimensions_apply_without_valid_monitors() {
        assert_eq!(virtual_desktop_size(&[]), (1920, 1080));
        assert_eq!(
            virtual_desktop_size(&[rect(-5000, 0, 0, 200)]),
            (1920, 1080)
        );
    }

    #[test]
    fn both_clip_dimensions_must_be_strictly_below_half_the_desktop() {
        assert!(is_clip_rect_locked_for_game(rect(0, 0, 959, 539), &[]));
        assert!(!is_clip_rect_locked_for_game(rect(0, 0, 960, 539), &[]));
        assert!(!is_clip_rect_locked_for_game(rect(0, 0, 959, 540), &[]));
    }

    #[test]
    fn narrow_clip_in_either_dimension_indicates_lock() {
        assert!(is_clip_rect_locked_for_game(rect(0, 0, 16, 1080), &[]));
        assert!(is_clip_rect_locked_for_game(rect(0, 0, 1920, 16), &[]));
        assert!(is_clip_rect_locked_for_game(rect(0, 0, 0, 0), &[]));
        assert!(!is_clip_rect_locked_for_game(rect(0, 0, 17, 1080), &[]));
        assert!(!is_clip_rect_locked_for_game(rect(0, 0, -1, 1080), &[]));
    }
}
