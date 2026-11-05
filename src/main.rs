//! Padd: a light, fast editor for Markdown, structured text and encrypted secrets.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod app;
mod crypto;
mod fileio;
mod highlight;
mod markdown;
mod secrets;
mod settings;
mod theme;

/// 128×128 RGBA app icon, rendered from assets/padd.svg.
pub const ICON_RGBA: &[u8] = include_bytes!("../assets/icon-128.rgba");

fn main() -> eframe::Result {
    let files: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(Into::into).collect();
    let size = settings::Settings::load().window.unwrap_or((1040.0, 720.0));
    let icon = eframe::egui::IconData {
        rgba: ICON_RGBA.to_vec(),
        width: 128,
        height: 128,
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Padd")
            .with_app_id("padd")
            .with_icon(std::sync::Arc::new(icon))
            .with_inner_size([size.0.max(360.0), size.1.max(240.0)])
            .with_min_inner_size([360.0, 240.0])
            .with_drag_and_drop(true),
        // No MSAA, no depth/stencil: egui needs none of them.
        multisampling: 0,
        depth_buffer: 0,
        stencil_buffer: 0,
        ..Default::default()
    };
    eframe::run_native(
        "Padd",
        options,
        Box::new(move |cc| Ok(Box::new(app::PaddApp::new(cc, files)))),
    )
}
