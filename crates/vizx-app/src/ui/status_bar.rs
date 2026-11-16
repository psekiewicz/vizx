use eframe::egui;

use crate::app::VizxApp;
use crate::theme;

pub fn show(ui: &mut egui::Ui, app: &VizxApp) {
    let project = app.session.project();
    let save_state = if app.session.has_unsaved_changes() {
        "Unsaved changes"
    } else {
        "No changes"
    };

    let file_location = match app.session.file_path() {
        Some(path) => path.display().to_string(),
        None => "Not saved".to_owned(),
    };

    ui.horizontal(|ui| {
        ui.label(muted(&app.status_message));
        ui.separator();
        ui.label(muted(&file_location));
        ui.separator();
        ui.label(muted(save_state));

        // Items added inside a right-to-left layout are placed starting from
        // the right edge, so the first one below ends up rightmost.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(muted(&format!("v{}", vizx_core::APP_VERSION)));
            if let Some(progress) = app.prerender_progress() {
                ui.separator();
                let megabytes = progress.stored_bytes as f64 / (1024.0 * 1024.0);
                let text = if progress.finished_frames < progress.total_frames {
                    format!(
                        "Pre-render {}%",
                        u64::from(progress.finished_frames) * 100
                            / u64::from(progress.total_frames.max(1))
                    )
                } else {
                    format!("Pre-render ready ({megabytes:.0} MB)")
                };
                ui.label(muted(&text)).on_hover_text(format!(
                    "Pre-render: {} / {} frames, {megabytes:.0} MB on disk",
                    progress.finished_frames, progress.total_frames
                ));
            }
            ui.separator();
            ui.label(muted(&app.graphics_summary))
                .on_hover_text("Graphics backend / GPU");
            ui.separator();
            ui.label(muted(&format!(
                "{} x {} @ {:.2} fps",
                project.settings.width,
                project.settings.height,
                project.settings.frame_rate.as_f64()
            )));
        });
    });
}

fn muted(text: &str) -> egui::RichText {
    egui::RichText::new(text).small().color(theme::TEXT_MUTED)
}
