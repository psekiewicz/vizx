//! VizX desktop application entry point.

// In release builds on Windows, do not open a console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod graphics;
mod theme;
mod transport;
mod ui;
mod visual_preview;

use eframe::egui;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(vizx_core::APP_NAME)
            // Size used when the window is not maximized. The app maximizes
            // itself on the first frame, see `VizxApp::maximize_on_startup`.
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 600.0]),
        wgpu_options: graphics::wgpu_configuration(),
        ..Default::default()
    };

    eframe::run_native(
        vizx_core::APP_NAME,
        native_options,
        Box::new(|creation_context| Ok(Box::new(app::VizxApp::new(creation_context)))),
    )
}
