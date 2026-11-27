//! Programmatic TouchPad Remote application icon (RGBA).

/// Generates an anti-aliased RGBA icon matching the dark cyan/emerald rounded touchpad motif.
pub fn generate_app_icon_rgba(size: u32) -> Vec<u8> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    if size == 0 {
        return rgba;
    }

    let s = size as f32;
    let pad = s * 0.08;
    let rect_size = s - pad * 2.0;
    let half_rect = rect_size * 0.5;
    let center = s * 0.5;
    let corner_radius = (s * 0.28 * 0.5).min(half_rect);
    let border_width = (s * 0.07).max(1.5);

    let dot_radius = s * 0.12;
    let dot_cy = s * 0.30 + dot_radius;

    let bar_w = s * 0.44;
    let bar_h = s * 0.12;
    let bar_cy = s * 0.65 + bar_h * 0.5;

    let bg_canvas = [0x0B_u8, 0x11, 0x20];
    let accent_cyan = [0x38_u8, 0xBD, 0xF8];
    let status_emerald = [0x10_u8, 0xB9, 0x81];

    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;

            let rect_sdf = sd_rounded_box(px - center, py - center, half_rect, half_rect, corner_radius);
            let outer_cov = coverage(rect_sdf);
            if outer_cov <= 0.0 {
                continue;
            }

            let mut color = [
                bg_canvas[0] as f32,
                bg_canvas[1] as f32,
                bg_canvas[2] as f32,
            ];

            // Cyan border around the inner touchpad tile.
            let border_sdf = rect_sdf.abs() - border_width * 0.5;
            let border_cov = coverage(border_sdf) * outer_cov;
            blend_rgb(&mut color, accent_cyan, border_cov);

            // Cyan center finger-dot.
            let dx = px - center;
            let dy = py - dot_cy;
            let dot_sdf = (dx * dx + dy * dy).sqrt() - dot_radius;
            let dot_cov = coverage(dot_sdf);
            blend_rgb(&mut color, accent_cyan, dot_cov);

            // Emerald click-bar pill at the bottom of the touchpad.
            let bar_sdf = sd_rounded_box(
                px - center,
                py - bar_cy,
                bar_w * 0.5,
                bar_h * 0.5,
                bar_h * 0.5,
            );
            let bar_cov = coverage(bar_sdf);
            blend_rgb(&mut color, status_emerald, bar_cov);

            let idx = ((y * size + x) * 4) as usize;
            rgba[idx] = color[0].round().clamp(0.0, 255.0) as u8;
            rgba[idx + 1] = color[1].round().clamp(0.0, 255.0) as u8;
            rgba[idx + 2] = color[2].round().clamp(0.0, 255.0) as u8;
            rgba[idx + 3] = (outer_cov * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }

    rgba
}

/// Builds an [`egui::IconData`] for the native window icon.
pub fn egui_icon_data(size: u32) -> egui::IconData {
    egui::IconData {
        rgba: generate_app_icon_rgba(size),
        width: size,
        height: size,
    }
}

fn sd_rounded_box(px: f32, py: f32, half_w: f32, half_h: f32, radius: f32) -> f32 {
    let qx = px.abs() - (half_w - radius);
    let qy = py.abs() - (half_h - radius);
    let out_x = qx.max(0.0);
    let out_y = qy.max(0.0);
    (out_x * out_x + out_y * out_y).sqrt() + qx.max(qy).min(0.0) - radius
}

fn coverage(signed_distance: f32) -> f32 {
    (0.5 - signed_distance).clamp(0.0, 1.0)
}

fn blend_rgb(dst: &mut [f32; 3], src: [u8; 3], alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let inv = 1.0 - alpha;
    dst[0] = dst[0] * inv + src[0] as f32 * alpha;
    dst[1] = dst[1] * inv + src[1] as f32 * alpha;
    dst[2] = dst[2] * inv + src[2] as f32 * alpha;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel_at(rgba: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let idx = ((y * size + x) * 4) as usize;
        [rgba[idx], rgba[idx + 1], rgba[idx + 2], rgba[idx + 3]]
    }

    #[test]
    fn generates_expected_icon_dimensions_and_colors() {
        let size = 32;
        let rgba = generate_app_icon_rgba(size);
        assert_eq!(rgba.len(), (size * size * 4) as usize);

        // Outer corner is transparent.
        assert_eq!(pixel_at(&rgba, size, 0, 0)[3], 0);

        // Center of the cyan dot (`y ≈ 0.42 * 32 = 13`) is cyan and opaque.
        let dot = pixel_at(&rgba, size, 16, 13);
        assert_eq!(dot, [0x38, 0xBD, 0xF8, 255]);

        // Center of the emerald click bar (`y ≈ 0.71 * 32 = 22`) is emerald and opaque.
        let bar = pixel_at(&rgba, size, 16, 22);
        assert_eq!(bar, [0x10, 0xB9, 0x81, 255]);

        let egui_icon = egui_icon_data(64);
        assert_eq!(egui_icon.width, 64);
        assert_eq!(egui_icon.height, 64);
        assert_eq!(egui_icon.rgba.len(), 64 * 64 * 4);
    }
}
