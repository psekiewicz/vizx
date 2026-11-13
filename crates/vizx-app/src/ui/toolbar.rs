use eframe::egui;

use crate::app::{AppCommand, VizxApp};
use crate::theme;
use crate::ui::NOT_AVAILABLE_YET;

pub fn show(ui: &mut egui::Ui, app: &mut VizxApp) {
    ui.horizontal(|ui| {
        command_button(
            ui,
            app,
            "New",
            "New project (Ctrl+N)",
            AppCommand::NewProject,
        );
        command_button(
            ui,
            app,
            "Open",
            "Open project (Ctrl+O)",
            AppCommand::OpenProject,
        );
        command_button(
            ui,
            app,
            "Save",
            "Save project (Ctrl+S)",
            AppCommand::SaveProject,
        );

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

fn command_button(
    ui: &mut egui::Ui,
    app: &mut VizxApp,
    label: &str,
    tooltip: &str,
    command: AppCommand,
) {
    if ui.button(label).on_hover_text(tooltip).clicked() {
        app.request(command);
    }
}

fn unavailable_button(ui: &mut egui::Ui, label: &str) {
    ui.add_enabled(false, egui::Button::new(label))
        .on_disabled_hover_text(NOT_AVAILABLE_YET);
}
