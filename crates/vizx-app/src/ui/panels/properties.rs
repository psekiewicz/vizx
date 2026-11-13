use eframe::egui;
use vizx_core::EditorSession;
use vizx_project::FrameRate;

use crate::theme;

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

/// Shows properties of the current selection.
///
/// Timeline objects do not exist yet, so nothing can be selected and the
/// panel shows the properties of the project itself. These fields edit the
/// real project data through [`EditorSession::edit_project`].
pub fn show(ui: &mut egui::Ui, session: &mut EditorSession) {
    // Work on a copy and compare afterwards: this is a simple way to know
    // whether any widget changed something during this frame.
    let mut edited = session.project().clone();

    section_heading(ui, "Project");
    egui::Grid::new("project_properties")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut edited.metadata.name);
            ui.end_row();

            ui.label("Author");
            ui.text_edit_singleline(&mut edited.metadata.author);
            ui.end_row();

            ui.label("Description");
            ui.text_edit_multiline(&mut edited.metadata.description);
            ui.end_row();
        });

    ui.add_space(8.0);
    section_heading(ui, "Composition");
    egui::Grid::new("composition_properties")
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
        });

    ui.add_space(8.0);
    section_heading(ui, "Selection");
    ui.label(
        egui::RichText::new("No object selected")
            .small()
            .color(theme::TEXT_MUTED),
    );

    session.edit_project(|project| {
        let has_changed = *project != edited;
        if has_changed {
            *project = edited;
        }
        has_changed
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

fn section_heading(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title).strong());
    ui.separator();
}
