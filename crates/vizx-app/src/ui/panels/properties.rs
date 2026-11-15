use eframe::egui;
use vizx_core::{AudioStore, EditorSession, ParameterKind, VisualizerCatalog};
use vizx_project::{
    AudioInput, AudioReaction, Clip, Easing, FrameAnimation, FrameLayout, MediaId, Placement,
    ReactionBand, TextAlignment, TextProperties, Track,
};

use crate::app::AppCommand;

use crate::theme;
use crate::ui::format_duration;

/// Shows the properties of whatever is selected in the editor.
///
/// This panel is only about the current selection. Settings of the project
/// as a whole live in the Project Settings window instead.
pub fn show(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    catalog: &mut VisualizerCatalog,
    audio_store: &AudioStore,
    requested_command: &mut Option<AppCommand>,
) {
    // A clip with many settings can be taller than the panel.
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            // The track is cloned so the session is free to be modified below.
            match session.selected_track().cloned() {
                Some(track) => {
                    track_properties(ui, session, catalog, audio_store, requested_command, &track)
                }
                None => nothing_selected(ui),
            }
        });
}

fn track_properties(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    catalog: &mut VisualizerCatalog,
    audio_store: &AudioStore,
    requested_command: &mut Option<AppCommand>,
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
        // With one clip selected, only that clip is shown; with the whole
        // track selected, all of its clips.
        let selected_clip = session.selected_clip_id();
        let shown_clips = track
            .clips
            .iter()
            .filter(|clip| selected_clip.is_none_or(|id| id == clip.id));
        for clip in shown_clips {
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
                        None if source.video().is_some() => "Video".to_owned(),
                        None if source.text().is_some() => "Text".to_owned(),
                        None if source.animation().is_some() => "Frame animation".to_owned(),
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
            let audio_media = source.audio().is_some().then_some(source.id);
            let animation = source
                .animation()
                .cloned()
                .map(|animation| (source.id, animation));
            // Cloned, so the session is free to be modified by the editor.
            let text = source.text().cloned().map(|text| (source.id, text));
            ui.label(source.display_name()).on_hover_text(&source.path);
            ui.label(
                egui::RichText::new(details)
                    .small()
                    .color(theme::TEXT_MUTED),
            );
            timing_editor(ui, session, clip);
            if let Some((media_id, animation)) = animation {
                animation_editor(ui, session, media_id, &animation);
            }
            if let Some(media_id) = audio_media {
                beats_editor(ui, session, audio_store, media_id);
            }
            // What the text says comes first; where and how it is shown
            // follows, as for every other element.
            if let Some((media_id, text)) = text {
                text_editor(ui, session, requested_command, media_id, &text);
            }
            if is_visualizer {
                audio_input_selector(ui, session, clip);
            }
            if is_visual {
                transform_editor(ui, session, clip);
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

    // How strongly the clip hears that audio. This is what calms a
    // visualizer down: it gets a quieter sound.
    let (quietest, loudest) = Clip::AUDIO_GAIN_RANGE;
    let mut percent = clip.audio_gain * 100.0;
    ui.horizontal(|ui| {
        ui.label("Sensitivity").on_hover_text(
            "How strongly this element reacts to the sound. Lower it if a visualizer is too \
             wild; raise it for quiet material. It does not change what you hear.",
        );
        ui.add(
            egui::Slider::new(&mut percent, quietest * 100.0..=loudest * 100.0)
                .logarithmic(true)
                .max_decimals(0)
                .suffix(" %"),
        );
    });
    if (percent / 100.0 - clip.audio_gain).abs() > 1e-6 {
        session.edit_project(|project| project.set_clip_audio_gain(clip.id, percent / 100.0));
    }
}

/// Where and how a visual clip is shown: position and size (percentages of
/// the picture), rotation, opacity and steady spin, plus its keyframes.
///
/// The values are those at the playhead. Without keyframes they hold for
/// the whole clip; with keyframes, changing a value changes the keyframe at
/// the playhead, or makes one there. Dragging the clip in the preview does
/// the same.
fn transform_editor(ui: &mut egui::Ui, session: &mut EditorSession, clip: &Clip) {
    let playhead_milliseconds = (session.playhead_seconds() * 1000.0).round() as u64;
    let moment = playhead_milliseconds
        .saturating_sub(clip.start_milliseconds)
        .min(clip.duration_milliseconds);
    let is_playhead_inside = playhead_milliseconds >= clip.start_milliseconds
        && playhead_milliseconds <= clip.end_milliseconds();
    let current = clip.transform_at(moment as f64);
    let keyframe_here = clip.keyframe_at(moment).copied();

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Transform").strong());
    ui.separator();
    let hint = match (clip.keyframes.is_empty(), is_playhead_inside) {
        (true, _) => "No keyframes: these values hold for the whole clip.".to_owned(),
        (false, true) if keyframe_here.is_some() => {
            "Values of the keyframe at the playhead.".to_owned()
        }
        (false, true) => format!(
            "{} keyframes. Changing a value adds one at the playhead.",
            clip.keyframes.len()
        ),
        (false, false) => {
            "The playhead is outside this clip; values at its nearest edge.".to_owned()
        }
    };
    ui.label(egui::RichText::new(hint).small().color(theme::TEXT_MUTED));

    let mut edited = current;
    egui::Grid::new(("transform", clip.id))
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
            percent_field(ui, "X", &mut edited.placement.x);
            percent_field(ui, "Y", &mut edited.placement.y);
            ui.end_row();
            percent_field(ui, "Width", &mut edited.placement.width);
            percent_field(ui, "Height", &mut edited.placement.height);
            ui.end_row();
            ui.label("Rotation");
            ui.add(
                egui::DragValue::new(&mut edited.rotation_degrees)
                    .speed(0.5)
                    .max_decimals(1)
                    .suffix("\u{b0}"),
            );
            percent_field(ui, "Opacity", &mut edited.opacity);
            ui.end_row();
        });
    edited.opacity = edited.opacity.clamp(0.0, 1.0);
    if ui
        .add_enabled(
            edited.placement != Placement::FULL,
            egui::Button::new("Fill Frame"),
        )
        .on_hover_text("Make the element cover the whole picture")
        .clicked()
    {
        edited.placement = Placement::FULL;
    }

    // Converting to percent and back can change the last digits; only a
    // real edit may touch the project.
    let values = |transform: &vizx_project::Transform| {
        [
            transform.placement.x,
            transform.placement.y,
            transform.placement.width,
            transform.placement.height,
            transform.rotation_degrees,
            transform.opacity,
        ]
    };
    let has_changed = values(&edited)
        .iter()
        .zip(values(&current))
        .any(|(new, old)| (new - old).abs() > 1e-9);
    if has_changed {
        session.edit_project(|project| {
            project.edit_clip_transform_at(clip.id, playhead_milliseconds, |transform| {
                *transform = edited;
            })
        });
    }

    // Keyframes.
    ui.horizontal(|ui| {
        match keyframe_here {
            Some(keyframe) => {
                if ui.button("Remove Keyframe").clicked() {
                    session.edit_project(|project| {
                        project.remove_keyframe(clip.id, playhead_milliseconds)
                    });
                }
                let mut easing = keyframe.easing;
                egui::ComboBox::from_id_salt(("easing", clip.id))
                    .selected_text(easing.label())
                    .show_ui(ui, |ui| {
                        for choice in Easing::ALL {
                            ui.selectable_value(&mut easing, choice, choice.label());
                        }
                    })
                    .response
                    .on_hover_text("How the element travels to this keyframe from the one before");
                if easing != keyframe.easing {
                    session.edit_project(|project| {
                        project.set_keyframe_easing(clip.id, playhead_milliseconds, easing)
                    });
                }
            }
            None => {
                let add = ui
                    .add_enabled(is_playhead_inside, egui::Button::new("Add Keyframe"))
                    .on_hover_text("Remember the values above at the playhead");
                if add.clicked() {
                    session.edit_project(|project| {
                        project.add_keyframe(clip.id, playhead_milliseconds)
                    });
                }
            }
        }

        // Jump to the keyframe before or after the playhead (within the clip).
        let visible: Vec<i64> = clip
            .keyframes
            .iter()
            .map(|keyframe| keyframe.time_milliseconds)
            .filter(|time| (0..=clip.duration_milliseconds as i64).contains(time))
            .collect();
        let here = moment as i64;
        let tolerance = vizx_project::KEYFRAME_TOLERANCE_MILLISECONDS as i64;
        let previous = visible
            .iter()
            .rev()
            .find(|time| **time < here - tolerance)
            .copied();
        let next = visible
            .iter()
            .find(|time| **time > here + tolerance)
            .copied();
        for (symbol, target, tooltip) in [
            ("\u{23EE}", previous, "Previous keyframe"),
            ("\u{23ED}", next, "Next keyframe"),
        ] {
            if ui
                .add_enabled(target.is_some(), egui::Button::new(symbol))
                .on_hover_text(tooltip)
                .clicked()
                && let Some(time) = target
            {
                session
                    .set_playhead_seconds((clip.start_milliseconds as i64 + time) as f64 / 1000.0);
            }
        }
        if !clip.keyframes.is_empty()
            && ui
                .button("Clear")
                .on_hover_text("Remove all keyframes; the element keeps the values at the playhead")
                .clicked()
        {
            session.edit_project(|project| project.clear_keyframes(clip.id, playhead_milliseconds));
        }
    });

    // Steady turning, on top of everything else.
    let mut spin = clip.spin_degrees_per_second;
    ui.horizontal(|ui| {
        ui.label("Spin")
            .on_hover_text("Keeps the element turning at a steady speed");
        ui.add(
            egui::DragValue::new(&mut spin)
                .speed(1.0)
                .max_decimals(1)
                .suffix("\u{b0}/s"),
        );
    });
    if (spin - clip.spin_degrees_per_second).abs() > 1e-9 {
        session.edit_project(|project| project.set_clip_spin(clip.id, spin));
    }
}
/// Where the beats of an audio file come from: found automatically, or
/// placed by hand (which replaces the found ones everywhere: animations,
/// visualizers and the beat marks on the timeline).
fn beats_editor(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    audio_store: &AudioStore,
    media_id: MediaId,
) {
    let Some(source) = session.project().media_source(media_id).cloned() else {
        return;
    };
    ui.add_space(6.0);
    ui.label(egui::RichText::new("Beats").strong());
    ui.separator();

    match &source.manual_beats {
        None => {
            let loaded = audio_store.loaded(&source.path);
            let found = loaded.map(|loaded| loaded.beats_at(source.tempo_bpm));
            let summary = match &found {
                Some((beats, _)) if source.tempo_bpm.is_some() => {
                    format!("{} beats at the tempo you chose", beats.len())
                }
                Some((beats, _)) => format!("Found automatically: {} beats", beats.len()),
                None => "Reading the file...".to_owned(),
            };
            ui.label(summary);

            // The tempo: found automatically, or set by the user. Music is
            // often felt at half or double the tempo found, so those are
            // one click away.
            if let Some((_, Some(tempo))) = found {
                let mut chosen = tempo;
                ui.horizontal(|ui| {
                    ui.label("Tempo").on_hover_text(
                        "The beats are placed at this tempo. If they come twice as often as \
                         you feel the music, halve it.",
                    );
                    ui.add(
                        egui::DragValue::new(&mut chosen)
                            .range(30.0..=300.0)
                            .speed(0.5)
                            .max_decimals(1)
                            .suffix(" BPM"),
                    );
                    if ui
                        .button("\u{bd}")
                        .on_hover_text("Half the tempo")
                        .clicked()
                    {
                        chosen = tempo / 2.0;
                    }
                    if ui
                        .button("\u{d7}2")
                        .on_hover_text("Double the tempo")
                        .clicked()
                    {
                        chosen = tempo * 2.0;
                    }
                    if source.tempo_bpm.is_some()
                        && ui
                            .button("Auto")
                            .on_hover_text("Use the tempo found automatically")
                            .clicked()
                    {
                        session.edit_project(|project| project.set_audio_tempo(media_id, None));
                    }
                });
                if (chosen - tempo).abs() > 1e-6 {
                    session.edit_project(|project| project.set_audio_tempo(media_id, Some(chosen)));
                }
            }

            ui.horizontal(|ui| {
                let edit = ui
                    .add_enabled(found.is_some(), egui::Button::new("Edit These Beats"))
                    .on_hover_text("Start from these beats and correct them by hand");
                if edit.clicked()
                    && let Some((beats, _)) = &found
                {
                    let seconds = beats.iter().map(|beat| beat.seconds).collect();
                    session.edit_project(|project| project.use_manual_beats(media_id, seconds));
                }
                if ui
                    .button("Draw My Own")
                    .on_hover_text("Start with no beats and place every beat yourself")
                    .clicked()
                {
                    session.edit_project(|project| project.use_manual_beats(media_id, Vec::new()));
                }
            });
        }
        Some(beats) => {
            let tempo = vizx_core::tempo_of(beats)
                .map_or(String::new(), |tempo| format!(", about {tempo:.0} BPM"));
            ui.label(format!("Drawn by hand: {} beats{tempo}", beats.len()));
            ui.label(
                egui::RichText::new(
                    "Click the strip along the bottom of the selected clip to add a beat, \
                     click a mark to remove it. Press B during playback to tap beats in.",
                )
                .small()
                .color(theme::TEXT_MUTED),
            );
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!beats.is_empty(), egui::Button::new("Clear"))
                    .clicked()
                {
                    session.edit_project(|project| project.use_manual_beats(media_id, Vec::new()));
                }
                if ui
                    .button("Back to Automatic")
                    .on_hover_text(
                        "Forget the beats drawn by hand and use the ones found automatically",
                    )
                    .clicked()
                {
                    session.edit_project(|project| project.use_automatic_beats(media_id));
                }
            });
        }
    }
}

/// How a frame animation is played, and for a sprite sheet its grid.
fn animation_editor(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    media_id: MediaId,
    animation: &FrameAnimation,
) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new("Animation").strong());
    ui.separator();
    let mut edited = animation.clone();
    let kind = match animation.layout {
        FrameLayout::Sequence { .. } => "Image sequence",
        FrameLayout::SpriteSheet { .. } => "Sprite sheet",
    };
    ui.label(
        egui::RichText::new(format!(
            "{kind}: {} frames of {} x {}",
            animation.frame_count, animation.frame_width, animation.frame_height
        ))
        .small()
        .color(theme::TEXT_MUTED),
    );

    egui::Grid::new(("animation", media_id))
        .num_columns(2)
        .show(ui, |ui| {
            if let FrameLayout::SpriteSheet { columns, rows, .. } = &mut edited.layout {
                ui.label("Columns");
                ui.add(egui::DragValue::new(columns).range(1..=256));
                ui.end_row();
                ui.label("Rows");
                ui.add(egui::DragValue::new(rows).range(1..=256));
                ui.end_row();
                ui.label("Frames").on_hover_text(
                    "How many cells hold a frame, counted left to right, top to bottom",
                );
                ui.add(egui::DragValue::new(&mut edited.frame_count).range(1..=65_536));
                ui.end_row();
            }
            ui.label("Speed");
            let (slowest, fastest) = FrameAnimation::SPEED_RANGE;
            ui.add_enabled(
                !edited.on_beat,
                egui::DragValue::new(&mut edited.frames_per_second)
                    .range(slowest..=fastest)
                    .speed(0.1)
                    .max_decimals(1)
                    .suffix(" fps"),
            );
            ui.end_row();
            ui.label("On beats")
                .on_hover_text("Step one frame on every beat instead of at a steady speed");
            ui.checkbox(&mut edited.on_beat, "");
            ui.end_row();
        });

    // When the grid changes, all of its cells are frames again.
    if let (
        FrameLayout::SpriteSheet { columns, rows, .. },
        FrameLayout::SpriteSheet {
            columns: old_columns,
            rows: old_rows,
            ..
        },
    ) = (edited.layout, animation.layout)
        && (columns, rows) != (old_columns, old_rows)
    {
        edited.frame_count = columns * rows;
    }
    if edited != *animation {
        session.edit_project(|project| project.set_animation(media_id, edited));
    }
}

/// When a clip starts and how long it lasts, as numbers. The same values
/// change when the clip is dragged or trimmed on the timeline.
fn timing_editor(ui: &mut egui::Ui, session: &mut EditorSession, clip: &Clip) {
    let mut start_seconds = clip.start_milliseconds as f64 / 1000.0;
    let mut length_seconds = clip.duration_milliseconds as f64 / 1000.0;
    ui.horizontal(|ui| {
        ui.label("Start");
        ui.add(
            egui::DragValue::new(&mut start_seconds)
                .range(0.0..=36_000.0)
                .speed(0.05)
                .max_decimals(2)
                .suffix(" s"),
        );
        ui.label("Length");
        ui.add(
            egui::DragValue::new(&mut length_seconds)
                .range(0.1..=36_000.0)
                .speed(0.05)
                .max_decimals(2)
                .suffix(" s"),
        );
    });

    let new_start = (start_seconds * 1000.0).round() as u64;
    let new_length = (length_seconds * 1000.0).round() as u64;
    if new_start != clip.start_milliseconds {
        session.edit_project(|project| project.move_clip(clip.id, new_start));
    } else if new_length != clip.duration_milliseconds {
        // The length changes at the clip's right edge.
        session.edit_project(|project| {
            project.trim_clip_end(clip.id, clip.start_milliseconds + new_length)
        });
    }
}

/// What a text element says and how it looks.
fn text_editor(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    requested_command: &mut Option<AppCommand>,
    media_id: MediaId,
    text: &TextProperties,
) {
    let mut edited = text.clone();

    ui.add(
        egui::TextEdit::multiline(&mut edited.text)
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .hint_text("Text"),
    );
    egui::Grid::new(("text", media_id))
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Size")
                .on_hover_text("Height of the letters, in percent of the picture's height");
            let (smallest, _) = TextProperties::SIZE_RANGE;
            let mut percent = edited.size * 100.0;
            ui.add(egui::Slider::new(&mut percent, smallest * 100.0..=50.0).suffix(" %"));
            edited.size = percent / 100.0;
            ui.end_row();

            ui.label("Color");
            let mut color = edited.color.map(|part| part as f32);
            ui.color_edit_button_rgb(&mut color);
            // Only take over the color if it was really changed: converting to
            // the color button's smaller numbers and back alters the last
            // digits.
            if color != text.color.map(|part| part as f32) {
                edited.color = color.map(f64::from);
            }
            ui.end_row();

            ui.label("Align")
                .on_hover_text("How lines of different length are lined up");
            egui::ComboBox::from_id_salt(("text_alignment", media_id))
                .selected_text(edited.alignment.label())
                .show_ui(ui, |ui| {
                    for choice in TextAlignment::ALL {
                        ui.selectable_value(&mut edited.alignment, choice, choice.label());
                    }
                });
            ui.end_row();

            ui.label("Font");
            ui.horizontal(|ui| {
                let font_name = if edited.font_path.is_empty() {
                    "Built-in"
                } else {
                    edited
                        .font_path
                        .rsplit(['/', '\\'])
                        .next()
                        .unwrap_or(&edited.font_path)
                };
                ui.label(font_name).on_hover_text(&edited.font_path);
                if ui
                    .button("Choose...")
                    .on_hover_text("Use a font file (TrueType or OpenType) from this computer")
                    .clicked()
                {
                    // Opening a file dialog is an application command.
                    *requested_command = Some(AppCommand::ChooseTextFont);
                }
                if !edited.font_path.is_empty() && ui.button("Built-in").clicked() {
                    edited.font_path.clear();
                }
            });
            ui.end_row();
        });

    let size_changed = (edited.size - text.size).abs() > 1e-9;
    let has_changed = size_changed
        || edited.text != text.text
        || edited.color != text.color
        || edited.alignment != text.alignment
        || edited.font_path != text.font_path;
    if has_changed {
        // The size is compared with a tolerance; keep the stored one if the
        // slider only changed its last digits.
        if !size_changed {
            edited.size = text.size;
        }
        if let Err(problem) = session.set_text(media_id, edited) {
            ui.colored_label(ui.visuals().error_fg_color, problem);
        }
    }
}

/// How a visual clip moves with the music.
///
/// A visualizer already has a "Reacts to" list above; for anything else
/// (an image) the list is shown here, next to the reaction it belongs to.
fn opacity_and_reaction_editor(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    clip: &Clip,
    has_audio_input_above: bool,
) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new("Animation to Sound").strong())
        .on_hover_text("Makes this element grow, move, turn or appear with the music");
    ui.separator();
    if !has_audio_input_above {
        audio_input_selector(ui, session, clip);
    }

    let mut edited = clip.reaction;
    // Shown as percentages; stored as fractions.
    let percent_slider = |ui: &mut egui::Ui, value: &mut f64, lowest: f64, highest: f64| {
        let mut percent = *value * 100.0;
        ui.add(egui::Slider::new(&mut percent, lowest..=highest).suffix(" %"));
        *value = percent / 100.0;
    };
    egui::Grid::new(("reaction", clip.id))
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Follows");
            egui::ComboBox::from_id_salt(("reaction_band", clip.id))
                .selected_text(edited.band.label())
                .show_ui(ui, |ui| {
                    for choice in ReactionBand::ALL {
                        ui.selectable_value(&mut edited.band, choice, choice.label());
                    }
                });
            ui.end_row();

            ui.label("Pulse")
                .on_hover_text("How much larger the element gets when the sound is loud");
            percent_slider(ui, &mut edited.scale, 0.0, 100.0);
            ui.end_row();

            ui.label("Fade")
                .on_hover_text("How much of the element disappears when the sound is quiet");
            percent_slider(ui, &mut edited.fade, 0.0, 100.0);
            ui.end_row();

            ui.label("Move X").on_hover_text(
                "How far the element moves sideways when the sound is loud, in percent of the \
                 picture's width (negative: to the left)",
            );
            percent_slider(ui, &mut edited.move_x, -50.0, 50.0);
            ui.end_row();

            ui.label("Move Y").on_hover_text(
                "How far the element moves down when the sound is loud, in percent of the \
                 picture's height (negative: up)",
            );
            percent_slider(ui, &mut edited.move_y, -50.0, 50.0);
            ui.end_row();

            ui.label("Rotate")
                .on_hover_text("How far the element turns when the sound is loud");
            ui.add(egui::Slider::new(&mut edited.rotate_degrees, -180.0..=180.0).suffix("\u{b0}"));
            ui.end_row();

            ui.label("Release").on_hover_text(
                "How long the element takes to settle after a loud moment. 0 follows the sound \
                 exactly; more makes it bounce back smoothly",
            );
            ui.add(
                egui::Slider::new(&mut edited.release_seconds, 0.0..=1.0)
                    .max_decimals(2)
                    .suffix(" s"),
            );
            ui.end_row();
        });

    // Comparing with a small tolerance: converting to percent and back can
    // change the last digits without the user having touched anything.
    let amounts = |reaction: AudioReaction| {
        [
            reaction.scale,
            reaction.fade,
            reaction.move_x,
            reaction.move_y,
            reaction.rotate_degrees,
            reaction.sensitivity,
            reaction.release_seconds,
        ]
    };
    let has_changed = edited.band != clip.reaction.band
        || amounts(edited)
            .iter()
            .zip(amounts(clip.reaction))
            .any(|(new, old)| (new - old).abs() > 1e-6);
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
