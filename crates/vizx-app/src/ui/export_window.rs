//! The window that shows a running video export.

use eframe::egui;

use crate::app::VizxApp;
use crate::theme;

/// Shows the progress of the export, if there is one, and lets the user
/// cancel it. When the export has ended, the window says how it went and
/// stays until it is closed.
pub fn show(ctx: &egui::Context, app: &mut VizxApp) {
    let Some(export) = app.export() else {
        return;
    };
    let finished = export.finished_frame_count();
    let total = export.frame_count().max(1);
    let result = export.result();
    let output = export.output().display().to_string();

    let mut wants_cancel = false;
    let mut wants_close = false;
    egui::Window::new("Export Video")
        .collapsible(false)
        .resizable(false)
        .default_pos(ctx.content_rect().center() - egui::vec2(210.0, 80.0))
        .show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(
                egui::RichText::new(&output)
                    .small()
                    .color(theme::TEXT_MUTED),
            );
            ui.add_space(6.0);

            match &result {
                None => {
                    ui.add(
                        egui::ProgressBar::new(finished as f32 / total as f32)
                            .text(format!("Frame {finished} of {total}")),
                    );
                    ui.add_space(6.0);
                    if ui.button("Cancel").clicked() {
                        wants_cancel = true;
                    }
                }
                Some(Ok(())) => {
                    ui.label(format!("Done ({total} frames)"));
                    ui.add_space(6.0);
                    wants_close = ui.button("Close").clicked();
                }
                Some(Err(problem)) => {
                    ui.colored_label(ui.visuals().error_fg_color, "Export failed");
                    ui.label(problem);
                    ui.add_space(6.0);
                    wants_close = ui.button("Close").clicked();
                }
            }
        });

    if wants_cancel && let Some(export) = app.export() {
        export.cancel();
    }
    if wants_close {
        app.close_export();
    }
}
