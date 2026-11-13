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

    ui.horizontal(|ui| {
        ui.label(muted("Ready"));
        ui.separator();
        ui.label(muted(&project.metadata.name));
        ui.separator();
        ui.label(muted(save_state));

        // Items added inside a right-to-left layout are placed starting from
        // the right edge, so the first one below ends up rightmost.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(muted(&format!("v{}", vizx_core::APP_VERSION)));
            ui.separator();
            ui.label(muted(&app.graphics_summary))
                .on_hover_text("Graphics backend and adapter selected by wgpu");
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
