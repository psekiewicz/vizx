use eframe::egui;
use vizx_core::EditorSession;
use vizx_project::{MediaContent, Track};

use crate::theme;
use crate::ui::format_duration;

/// Shows the properties of whatever is selected in the editor.
///
/// This panel is only about the current selection. Settings of the project
/// as a whole live in the Project Settings window instead.
pub fn show(ui: &mut egui::Ui, session: &mut EditorSession) {
    // The track is cloned so the session is free to be modified below.
    match session.selected_track().cloned() {
        Some(track) => track_properties(ui, session, &track),
        None => nothing_selected(ui),
    }
}

fn track_properties(ui: &mut egui::Ui, session: &mut EditorSession, track: &Track) {
    ui.label(egui::RichText::new("Track").strong());
    ui.separator();

    let mut name = track.name.clone();
    egui::Grid::new("track_properties")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut name);
            ui.end_row();

            ui.label("Kind");
            ui.label(track.kind.label());
            ui.end_row();
        });
    session.edit_project(|project| project.rename_track(track.id, &name));

    if !track.clips.is_empty() {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("Content").strong());
        ui.separator();
        for clip in &track.clips {
            let Some(source) = session.project().media_source(clip.media) else {
                continue;
            };
            let MediaContent::Audio(audio) = &source.content;
            ui.label(source.file_name()).on_hover_text(&source.path);
            ui.label(
                egui::RichText::new(format!(
                    "Length {}  |  {} Hz  |  {} ch",
                    format_duration(clip.duration_milliseconds),
                    audio.sample_rate,
                    audio.channel_count
                ))
                .small()
                .color(theme::TEXT_MUTED),
            );
        }
    }

    ui.add_space(8.0);
    if ui.button("Delete Track").clicked() {
        session.remove_track(track.id);
    }
}

fn nothing_selected(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Nothing selected").color(theme::TEXT_MUTED));
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("Select a track in the timeline to edit it here.")
            .small()
            .color(theme::TEXT_MUTED),
    );
}
