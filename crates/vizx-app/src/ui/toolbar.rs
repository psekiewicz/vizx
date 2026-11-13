use eframe::egui;

use crate::app::VizxApp;
use crate::theme;
use crate::ui::NOT_AVAILABLE_YET;

pub fn show(ui: &mut egui::Ui, app: &mut VizxApp) {
    ui.horizontal(|ui| {
        if ui.button("New").on_hover_text("New project").clicked() {
            app.start_new_project();
        }
        unavailable_button(ui, "Open");
        unavailable_button(ui, "Save");

        ui.separator();

        // Transport controls. They become functional with the audio engine.
        unavailable_button(ui, "\u{23EE}"); // skip to start
        unavailable_button(ui, "\u{25B6}"); // play
        unavailable_button(ui, "\u{23F9}"); // stop
        unavailable_button(ui, "\u{23ED}"); // skip to end

        ui.separator();

        ui.label(
            egui::RichText::new("00:00:00.000")
                .monospace()
                .color(theme::TEXT_MUTED),
        )
        .on_hover_text("Playhead position (placeholder)");
    });
}

fn unavailable_button(ui: &mut egui::Ui, label: &str) {
    ui.add_enabled(false, egui::Button::new(label))
        .on_disabled_hover_text(NOT_AVAILABLE_YET);
}
