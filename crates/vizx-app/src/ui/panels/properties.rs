use eframe::egui;
use vizx_core::{EditorSession, ParameterKind, VisualizerCatalog};
use vizx_project::{AudioInput, AudioReaction, Clip, Placement, ReactionBand, Track};

use crate::theme;
use crate::ui::format_duration;

/// Shows the properties of whatever is selected in the editor.
///
/// This panel is only about the current selection. Settings of the project
/// as a whole live in the Project Settings window instead.
pub fn show(ui: &mut egui::Ui, session: &mut EditorSession, catalog: &mut VisualizerCatalog) {
    // A clip with many settings can be taller than the panel.
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            // The track is cloned so the session is free to be modified below.
            match session.selected_track().cloned() {
                Some(track) => track_properties(ui, session, catalog, &track),
                None => nothing_selected(ui),
            }
        });
}

fn track_properties(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    catalog: &mut VisualizerCatalog,
    track: &Track,
) {
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
                None => {
                    let kind = match source.image() {
                        Some(image) => format!("Image {} x {}", image.width, image.height),
                        None if source.is_milkdrop_preset() => "MilkDrop preset".to_owned(),
                        None => "Visualizer plugin".to_owned(),
                    };
                    format!(
                        "{kind}  |  Length {}",
                        format_duration(clip.duration_milliseconds)
                    )
                }
            };
            let is_visualizer = source.is_visualizer();
            let plugin_path = source.is_visualizer_plugin().then(|| source.path.clone());
            let is_visual = source.is_visual();
            ui.label(source.file_name()).on_hover_text(&source.path);
            ui.label(
                egui::RichText::new(details)
                    .small()
                    .color(theme::TEXT_MUTED),
            );
            if is_visualizer {
                audio_input_selector(ui, session, clip);
            }
            if is_visual {
                placement_editor(ui, session, clip);
                opacity_and_reaction_editor(ui, session, clip, is_visualizer);
            }
            if let Some(plugin_path) = plugin_path {
                plugin_parameters(ui, session, catalog, clip, &plugin_path);
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

/// The opacity of a visual clip and how it moves with the music.
///
/// A visualizer already has a "Reacts to" list above; for anything else
/// (an image) the list is shown here, next to the reaction it belongs to.
fn opacity_and_reaction_editor(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    clip: &Clip,
    has_audio_input_above: bool,
) {
    let mut opacity_percent = clip.opacity * 100.0;
    ui.horizontal(|ui| {
        ui.label("Opacity");
        ui.add(egui::Slider::new(&mut opacity_percent, 0.0..=100.0).suffix(" %"));
    });
    if (opacity_percent / 100.0 - clip.opacity).abs() > 1e-6 {
        session.edit_project(|project| project.set_clip_opacity(clip.id, opacity_percent / 100.0));
    }

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Reaction to Sound").strong())
        .on_hover_text("Makes this element grow or appear with the music");
    ui.separator();
    if !has_audio_input_above {
        audio_input_selector(ui, session, clip);
    }

    let mut band = clip.reaction.band;
    let mut pulse_percent = clip.reaction.scale * 100.0;
    let mut fade_percent = clip.reaction.fade * 100.0;
    egui::Grid::new(("reaction", clip.id))
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Follows");
            egui::ComboBox::from_id_salt(("reaction_band", clip.id))
                .selected_text(band.label())
                .show_ui(ui, |ui| {
                    for choice in ReactionBand::ALL {
                        ui.selectable_value(&mut band, choice, choice.label());
                    }
                });
            ui.end_row();

            ui.label("Pulse")
                .on_hover_text("How much larger the element gets when the sound is loud");
            ui.add(egui::Slider::new(&mut pulse_percent, 0.0..=100.0).suffix(" %"));
            ui.end_row();

            ui.label("Fade")
                .on_hover_text("How much of the element disappears when the sound is quiet");
            ui.add(egui::Slider::new(&mut fade_percent, 0.0..=100.0).suffix(" %"));
            ui.end_row();
        });

    let edited = AudioReaction {
        band,
        scale: pulse_percent / 100.0,
        fade: fade_percent / 100.0,
    };
    let has_changed = edited.band != clip.reaction.band
        || (edited.scale - clip.reaction.scale).abs() > 1e-6
        || (edited.fade - clip.reaction.fade).abs() > 1e-6;
    if has_changed {
        session.edit_project(|project| project.set_clip_reaction(clip.id, edited));
    }
}

/// The adjustable values of a visualizer plugin, as its `visualizer.json`
/// describes them: a slider for a number, a color button for a color, a
/// checkbox for a switch.
fn plugin_parameters(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    catalog: &mut VisualizerCatalog,
    clip: &Clip,
    plugin_path: &str,
) {
    // Cloned, so the catalog is not borrowed while the widgets are drawn.
    let manifest = match catalog.plugin(plugin_path).manifest.clone() {
        Ok(manifest) => manifest,
        Err(problem) => {
            ui.colored_label(ui.visuals().error_fg_color, problem);
            return;
        }
    };
    if manifest.parameters.is_empty() {
        return;
    }

    ui.add_space(6.0);
    ui.label(egui::RichText::new(&manifest.name).strong())
        .on_hover_text(&manifest.description);
    ui.separator();

    // (parameter id, new values) for everything the user changed this frame.
    let mut changes: Vec<(String, Vec<f64>)> = Vec::new();
    egui::Grid::new(("plugin_parameters", clip.id))
        .num_columns(2)
        .show(ui, |ui| {
            for parameter in &manifest.parameters {
                let current = parameter.sanitized(clip.parameters.get(&parameter.id));
                ui.label(&parameter.name);
                let edited = match &parameter.kind {
                    ParameterKind::Number { min, max, step, .. } => {
                        let mut value = current[0];
                        let mut slider = egui::Slider::new(&mut value, *min..=*max);
                        if let Some(step) = step {
                            slider = slider.step_by(*step);
                        }
                        ui.add(slider);
                        vec![value]
                    }
                    ParameterKind::Color { .. } => {
                        let mut color = [current[0] as f32, current[1] as f32, current[2] as f32];
                        ui.color_edit_button_rgb(&mut color);
                        color.iter().map(|part| f64::from(*part)).collect()
                    }
                    ParameterKind::Toggle { .. } => {
                        let mut is_on = current[0] >= 0.5;
                        ui.checkbox(&mut is_on, "");
                        vec![f64::from(u8::from(is_on))]
                    }
                };
                ui.end_row();

                let has_changed = edited
                    .iter()
                    .zip(&current)
                    .any(|(new, old)| (new - old).abs() > 1e-6);
                if has_changed {
                    changes.push((parameter.id.clone(), edited));
                }
            }
        });

    if ui
        .add_enabled(
            !clip.parameters.is_empty(),
            egui::Button::new("Reset to Defaults"),
        )
        .clicked()
    {
        session.edit_project(|project| project.reset_clip_parameters(clip.id));
        return;
    }
    for (id, values) in changes {
        session.edit_project(|project| project.set_clip_parameter(clip.id, &id, values));
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
