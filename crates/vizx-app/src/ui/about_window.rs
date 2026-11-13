use eframe::egui;

use crate::app::VizxApp;

/// Small "About" dialog. It is an egui window drawn inside the main window,
/// not a separate operating-system window.
pub fn show(ctx: &egui::Context, app: &mut VizxApp) {
    egui::Window::new("About VizX")
        .open(&mut app.is_about_window_open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.heading(vizx_core::APP_NAME);
            ui.label(format!("Version {}", vizx_core::APP_VERSION));
            ui.add_space(6.0);
            ui.label("Music-driven audiovisual composition editor.");
            ui.add_space(6.0);
            ui.label(format!("Graphics: {}", app.graphics_summary));
        });
}
