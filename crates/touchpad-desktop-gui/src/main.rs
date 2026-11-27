//! Native desktop GUI binary (`TouchPadRemote`) for Windows and Linux.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod client_bridge;
mod icon;
mod telemetry;
mod theme;
mod tray;

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

fn main() -> eframe::Result<()> {
    init_logging();

    let start_minimized = client_bridge::initial_start_minimized_to_tray();
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("TouchPad Remote")
            .with_app_id("touchpad-remote")
            .with_inner_size([860.0, 760.0])
            .with_min_inner_size([700.0, 620.0])
            .with_icon(icon::egui_icon_data(64))
            .with_visible(!start_minimized),
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "TouchPad Remote",
        native_options,
        Box::new(move |cc| {
            theme::apply_dark_theme(&cc.egui_ctx);
            Ok(Box::new(app::TouchPadGuiApp::new(
                cc.egui_ctx.clone(),
                start_minimized,
            )))
        }),
    )
}
