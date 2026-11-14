use eframe::egui;
use vizx_core::EditorSession;
use vizx_project::{AudioInput, Clip, Placement, Track};

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
            let details = match source.audio() {
                Some(audio) => format!(
                    "Length {}  |  {} Hz  |  {} ch",
                    format_duration(clip.duration_milliseconds),
                    audio.sample_rate,
                    audio.channel_count
                ),
                None => format!(
                    "MilkDrop preset  |  Length {}",
                    format_duration(clip.duration_milliseconds)
                ),
            };
            ui.label(source.file_name()).on_hover_text(&source.path);
            ui.label(
                egui::RichText::new(details)
                    .small()
                    .color(theme::TEXT_MUTED),
            );
            if source.is_milkdrop_preset() {
                audio_input_selector(ui, session, clip);
                placement_editor(ui, session, clip);
            }
        }
    }

    ui.add_space(8.0);
    if ui.button("Delete Track").clicked() {
        session.remove_track(track.id);
    }
}

/// Lets the user choose which audio a visualizer reacts to: everything, or
/// one audio track (for example only the vocals).
fn audio_input_selector(ui: &mut egui::Ui, session: &mut EditorSession, clip: &Clip) {
    const MIX_LABEL: &str = "All audio (mix)";

    // Collected first, so the session can be modified afterwards.
    let audio_tracks: Vec<(AudioInput, String)> = session
        .project()
        .audio_tracks()
        .map(|track| (AudioInput::Track(track.id), track.name.clone()))
        .collect();
    let current_label = audio_tracks
        .iter()
        .find(|(input, _)| *input == clip.audio_input)
        .map_or(MIX_LABEL, |(_, name)| name.as_str());

    let mut chosen = clip.audio_input;
    ui.horizontal(|ui| {
        ui.label("Reacts to")
            .on_hover_text("The audio this visualizer follows");
        egui::ComboBox::from_id_salt(("audio_input", clip.id))
            .selected_text(current_label)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen, AudioInput::Mix, MIX_LABEL);
                for (input, name) in &audio_tracks {
                    ui.selectable_value(&mut chosen, *input, name);
                }
            });
    });
    session.edit_project(|project| project.set_clip_audio_input(clip.id, chosen));
}

/// Position and size of a visual clip in the picture, as percentages of
/// the composition. The same values change when the clip is dragged in the
/// preview.
fn placement_editor(ui: &mut egui::Ui, session: &mut EditorSession, clip: &Clip) {
    let mut edited = clip.placement;

    egui::Grid::new(("placement", clip.id))
        .num_columns(4)
        .show(ui, |ui| {
            let percent_field = |ui: &mut egui::Ui, label: &str, value: &mut f64| {
                ui.label(label);
                let mut percent = *value * 100.0;
                ui.add(
                    egui::DragValue::new(&mut percent)
                        .speed(0.25)
                        .max_decimals(1)
                        .suffix(" %"),
                );
                *value = percent / 100.0;
            };
            percent_field(ui, "X", &mut edited.x);
            percent_field(ui, "Y", &mut edited.y);
            ui.end_row();
            percent_field(ui, "Width", &mut edited.width);
            percent_field(ui, "Height", &mut edited.height);
            ui.end_row();
        });
    if ui
        .add_enabled(
            clip.placement != Placement::FULL,
            egui::Button::new("Fill Frame"),
        )
        .on_hover_text("Make the visualizer cover the whole picture again")
        .clicked()
    {
        edited = Placement::FULL;
    }

    // Converting to percent and back can change the last digits; only a
    // real edit may touch the project.
    let has_changed = [
        (edited.x, clip.placement.x),
        (edited.y, clip.placement.y),
        (edited.width, clip.placement.width),
        (edited.height, clip.placement.height),
    ]
    .iter()
    .any(|(new, old)| (new - old).abs() > 1e-9);
    if has_changed {
        session.edit_project(|project| project.set_clip_placement(clip.id, edited));
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
