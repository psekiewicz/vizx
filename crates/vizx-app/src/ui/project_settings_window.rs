use eframe::egui;
use vizx_project::FrameRate;

use crate::app::VizxApp;

/// Frame rates offered in the drop-down list.
const FRAME_RATE_CHOICES: [(&str, FrameRate); 5] = [
    ("24", FrameRate::whole(24)),
    ("25", FrameRate::whole(25)),
    (
        "29.97",
        FrameRate {
            numerator: 30000,
            denominator: 1001,
        },
    ),
    ("30", FrameRate::FPS_30),
    ("60", FrameRate::FPS_60),
];

/// Largest composition edge accepted by the size fields, in pixels.
const MAX_COMPOSITION_EDGE: u32 = 8192;
/// Longest project length accepted by the duration field: ten hours.
const MAX_DURATION_SECONDS: f64 = 36_000.0;

/// Settings of the project as a whole, shown on demand.
///
/// It is opened from File > Project Settings or from a context menu, so
/// these rarely changed values do not take up permanent space in the
/// editor. It is an egui window drawn inside the main window, not a separate
/// operating-system window. Changes apply immediately.
pub fn show(ctx: &egui::Context, app: &mut VizxApp) {
    egui::Window::new("Project Settings")
        .open(&mut app.is_project_settings_open)
        .collapsible(false)
        .resizable(false)
        .default_pos(ctx.content_rect().center() - egui::vec2(180.0, 160.0))
        .show(ctx, |ui| {
            // Work on a copy and compare afterwards: a simple way to know
            // whether any widget changed something during this frame.
            let mut edited = app.session.project().clone();

            ui.label(egui::RichText::new("Project").strong());
            ui.separator();
            egui::Grid::new("project_settings_metadata")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Author");
                    ui.text_edit_singleline(&mut edited.metadata.author);
                    ui.end_row();

                    ui.label("Description");
                    ui.text_edit_multiline(&mut edited.metadata.description);
                    ui.end_row();
                });

            ui.add_space(8.0);
            ui.label(egui::RichText::new("Composition").strong());
            ui.separator();
            egui::Grid::new("project_settings_composition")
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Width");
                    ui.add(
                        egui::DragValue::new(&mut edited.settings.width)
                            .range(16..=MAX_COMPOSITION_EDGE)
                            .suffix(" px"),
                    );
                    ui.end_row();

                    ui.label("Height");
                    ui.add(
                        egui::DragValue::new(&mut edited.settings.height)
                            .range(16..=MAX_COMPOSITION_EDGE)
                            .suffix(" px"),
                    );
                    ui.end_row();

                    ui.label("Frame rate");
                    frame_rate_selector(ui, &mut edited.settings.frame_rate);
                    ui.end_row();

                    ui.label("Duration");
                    let mut duration_seconds = edited.settings.duration_seconds();
                    ui.add(
                        egui::DragValue::new(&mut duration_seconds)
                            .range(1.0..=MAX_DURATION_SECONDS)
                            .speed(1.0)
                            .max_decimals(3)
                            .suffix(" s"),
                    );
                    edited.settings.duration_milliseconds =
                        (duration_seconds * 1000.0).round() as u64;
                    ui.end_row();
                });

            app.session.edit_project(|project| {
                let has_changed = *project != edited;
                if has_changed {
                    *project = edited;
                }
                has_changed
            });
        });
}

fn frame_rate_selector(ui: &mut egui::Ui, frame_rate: &mut FrameRate) {
    let current_label = FRAME_RATE_CHOICES
        .iter()
        .find(|(_, choice)| choice == frame_rate)
        .map_or_else(
            || format!("{:.2}", frame_rate.as_f64()),
            |(label, _)| (*label).to_owned(),
        );

    egui::ComboBox::from_id_salt("frame_rate")
        .selected_text(format!("{current_label} fps"))
        .show_ui(ui, |ui| {
            for (label, choice) in FRAME_RATE_CHOICES {
                ui.selectable_value(frame_rate, choice, format!("{label} fps"));
            }
        });
}
