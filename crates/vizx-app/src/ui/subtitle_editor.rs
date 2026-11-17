//! The Subtitle Editor: every line of every subtitle layer in one table, to
//! fix words and times, move lines between layers (singers), split and
//! merge them, and shift a whole layer in time. Also the editor of a
//! subtitle layer's look, shown in the Properties panel.
//!
//! Lines are clips, so everything done here is also visible (and undoable)
//! on the timeline.

use eframe::egui;
use vizx_core::EditorSession;
use vizx_project::{
    ClipId, Placement, SubtitleFit, SubtitlePosition, SubtitleStyle, TextAlignment, Track, TrackId,
    TrackKind,
};

use crate::app::{AppCommand, VizxApp};
use crate::theme;

/// What the Subtitle Editor remembers while the app runs.
#[derive(Default)]
pub struct SubtitleEditor {
    pub is_open: bool,
    /// Only the lines of this layer; `None` shows all subtitle layers.
    pub layer: Option<TrackId>,
    /// The line last clicked.
    selected: Option<ClipId>,
    /// How far "Shift" moves the shown lines, in seconds.
    shift_seconds: f64,
    /// Lines shown contain this text (any case).
    search: String,
    /// Keep the line at the playhead in view while playing.
    follow_playhead: bool,
    message: String,
}

impl SubtitleEditor {
    /// Opens the editor on one layer (or all of them).
    pub fn open(&mut self, layer: Option<TrackId>) {
        self.is_open = true;
        self.layer = layer;
        self.follow_playhead = true;
    }
}

/// One line as the table shows it.
struct Row {
    clip_id: ClipId,
    track_id: TrackId,
    start: f64,
    end: f64,
    text: String,
}

pub fn show(ctx: &egui::Context, app: &mut VizxApp) {
    if !app.subtitle_editor.is_open {
        return;
    }
    let mut is_open = true;
    egui::Window::new("Subtitle Editor")
        .open(&mut is_open)
        .collapsible(false)
        .resizable(true)
        .default_size([640.0, 460.0])
        .show(ctx, |ui| contents(ui, app));
    app.subtitle_editor.is_open = is_open;
}

fn contents(ui: &mut egui::Ui, app: &mut VizxApp) {
    let layers: Vec<(TrackId, String)> = app
        .session
        .project()
        .tracks()
        .iter()
        .filter(|track| track.kind == TrackKind::Subtitles)
        .map(|track| (track.id, track.name.clone()))
        .collect();
    if layers.is_empty() {
        ui.label("There are no subtitle layers yet.");
        ui.label(
            egui::RichText::new(
                "Right-click the timeline and choose Add > Subtitle Layer, or add lyrics with \
                 Add > Lyrics / Subtitles. A layer of texts can also be converted (right-click \
                 its name).",
            )
            .small()
            .color(theme::TEXT_MUTED),
        );
        return;
    }
    let editor = &mut app.subtitle_editor;
    if editor
        .layer
        .is_some_and(|id| !layers.iter().any(|(layer, _)| *layer == id))
    {
        editor.layer = None;
    }

    // Which lines, and the tools for them.
    ui.horizontal(|ui| {
        ui.label("Layer");
        let current = editor
            .layer
            .and_then(|id| layers.iter().find(|(layer, _)| *layer == id))
            .map_or("All layers", |(_, name)| name.as_str());
        egui::ComboBox::from_id_salt("subtitle_editor_layer")
            .selected_text(current)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut editor.layer, None, "All layers");
                for (id, name) in &layers {
                    ui.selectable_value(&mut editor.layer, Some(*id), name);
                }
            });
        ui.add(
            egui::TextEdit::singleline(&mut editor.search)
                .hint_text("Search")
                .desired_width(120.0),
        );
        ui.checkbox(&mut editor.follow_playhead, "Follow playhead");
    });

    let playhead = app.session.playhead_seconds();
    let rows = rows(
        &app.session,
        app.subtitle_editor.layer,
        &app.subtitle_editor.search,
    );
    line_tools(ui, app, &rows, &layers, playhead);
    ui.separator();
    table(ui, app, &rows, &layers, playhead);

    let editor = &app.subtitle_editor;
    if !editor.message.is_empty() {
        ui.label(
            egui::RichText::new(&editor.message)
                .small()
                .color(theme::TEXT_MUTED),
        );
    }
}

fn rows(session: &EditorSession, layer: Option<TrackId>, search: &str) -> Vec<Row> {
    let project = session.project();
    let search = search.to_lowercase();
    let mut rows: Vec<Row> = project
        .tracks()
        .iter()
        .filter(|track| track.kind == TrackKind::Subtitles)
        .filter(|track| layer.is_none_or(|id| id == track.id))
        .flat_map(|track| {
            track.clips.iter().map(|clip| Row {
                clip_id: clip.id,
                track_id: track.id,
                start: clip.start_milliseconds as f64 / 1000.0,
                end: clip.end_milliseconds() as f64 / 1000.0,
                text: project.clip_text(clip.id).unwrap_or_default().to_owned(),
            })
        })
        .filter(|row| search.is_empty() || row.text.to_lowercase().contains(&search))
        .collect();
    rows.sort_by(|a, b| a.start.total_cmp(&b.start));
    rows
}

/// Buttons that work on the selected line or on all lines shown.
fn line_tools(
    ui: &mut egui::Ui,
    app: &mut VizxApp,
    rows: &[Row],
    layers: &[(TrackId, String)],
    playhead: f64,
) {
    let selected = app
        .subtitle_editor
        .selected
        .and_then(|id| rows.iter().find(|row| row.clip_id == id));
    let target_layer = selected
        .map(|row| row.track_id)
        .or(app.subtitle_editor.layer)
        .unwrap_or(layers[0].0);
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Add Line at Playhead")
            .on_hover_text("A new 2-second line on the selected line's layer")
            .clicked()
        {
            let start = milliseconds(playhead);
            let mut added = None;
            app.session.edit_project(|project| {
                added = project.add_subtitle_line(target_layer, "New line", start, 2000);
                added.is_some()
            });
            app.subtitle_editor.selected = added;
        }
        let selected_id = selected.map(|row| row.clip_id);
        let inside = selected.is_some_and(|row| row.start < playhead && playhead < row.end);
        if ui
            .add_enabled(inside, egui::Button::new("Split at Playhead"))
            .on_hover_text("Cut the selected line in two at the playhead; the words are divided")
            .clicked()
            && let Some(row) = selected
        {
            split_line(app, row, playhead);
        }
        let next = selected.and_then(|row| {
            rows.iter()
                .find(|other| other.track_id == row.track_id && other.start >= row.end - 1e-6)
        });
        if ui
            .add_enabled(next.is_some(), egui::Button::new("Merge with Next"))
            .on_hover_text("Join the selected line and the next one on its layer")
            .clicked()
            && let (Some(row), Some(next)) = (selected, next)
        {
            let joined = format!("{} {}", row.text.trim_end(), next.text.trim_start());
            let (first, second, end) = (row.clip_id, next.clip_id, milliseconds(next.end));
            app.session.edit_project(|project| {
                project.trim_clip_end(first, end);
                project.set_clip_text(first, &joined);
                project.remove_clip(second)
            });
        }
        if ui
            .add_enabled(selected_id.is_some(), egui::Button::new("Delete Line"))
            .clicked()
            && let Some(id) = selected_id
        {
            app.session.edit_project(|project| project.remove_clip(id));
            app.subtitle_editor.selected = None;
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label("Shift shown lines by");
        ui.add(
            egui::DragValue::new(&mut app.subtitle_editor.shift_seconds)
                .range(-60.0..=60.0)
                .speed(0.01)
                .max_decimals(2)
                .suffix(" s"),
        );
        let shift = app.subtitle_editor.shift_seconds;
        if ui
            .add_enabled(shift != 0.0 && !rows.is_empty(), egui::Button::new("Shift"))
            .on_hover_text("Move every line in the table earlier (-) or later (+)")
            .clicked()
        {
            let moves: Vec<(ClipId, u64)> = rows
                .iter()
                .map(|row| (row.clip_id, milliseconds(row.start + shift)))
                .collect();
            app.session.edit_project(|project| {
                for (clip, start) in &moves {
                    project.move_clip(*clip, *start);
                }
                true
            });
            app.subtitle_editor.message = format!("{} lines shifted by {shift:+.2} s", rows.len());
        }
        if ui
            .add_enabled(!rows.is_empty(), egui::Button::new("Save as .srt..."))
            .on_hover_text("The lines in the table as a subtitle file")
            .clicked()
        {
            save_srt(app, rows);
        }
        if ui
            .button("Import .lrc / .srt...")
            .on_hover_text("Opens the Lyrics window's import")
            .clicked()
        {
            app.request(AppCommand::ImportLyricsFile);
        }
    });
}

/// The table of lines: times, layer and words, all editable.
fn table(
    ui: &mut egui::Ui,
    app: &mut VizxApp,
    rows: &[Row],
    layers: &[(TrackId, String)],
    playhead: f64,
) {
    let current = rows
        .iter()
        .position(|row| row.start <= playhead && playhead < row.end);
    let row_height = ui.spacing().interact_size.y + 4.0;
    let mut scroll = egui::ScrollArea::vertical().auto_shrink(false);
    if app.subtitle_editor.follow_playhead
        && app.is_playing()
        && let Some(index) = current
    {
        scroll = scroll.vertical_scroll_offset((index as f32 - 3.0).max(0.0) * row_height);
    }
    scroll.show(ui, |ui| {
        egui::Grid::new("subtitle_lines")
            .num_columns(6)
            .striped(true)
            .min_row_height(row_height)
            .show(ui, |ui| {
                ui.label("");
                ui.label(egui::RichText::new("Start").strong());
                ui.label(egui::RichText::new("End").strong());
                ui.label(egui::RichText::new("Layer").strong());
                ui.label(egui::RichText::new("Words").strong());
                ui.end_row();
                for (index, row) in rows.iter().enumerate() {
                    line_row(ui, app, row, layers, current == Some(index), playhead);
                    ui.end_row();
                }
            });
    });
}

fn line_row(
    ui: &mut egui::Ui,
    app: &mut VizxApp,
    row: &Row,
    layers: &[(TrackId, String)],
    is_current: bool,
    playhead: f64,
) {
    let is_selected = app.subtitle_editor.selected == Some(row.clip_id);
    let marker = if is_current { "▶" } else { "  " };
    if ui
        .selectable_label(is_selected, marker)
        .on_hover_text("Select this line and move the playhead to its start")
        .clicked()
    {
        app.subtitle_editor.selected = Some(row.clip_id);
        app.session.set_playhead_seconds(row.start);
    }

    // Start: changing it keeps the end where it is.
    ui.horizontal(|ui| {
        let mut start = row.start;
        if ui.add(time_field(&mut start)).changed() {
            let new_start = milliseconds(start.min(row.end - 0.05));
            app.session
                .edit_project(|project| project.trim_clip_start(row.clip_id, new_start));
        }
        if ui
            .small_button("⏷")
            .on_hover_text("Start at the playhead")
            .clicked()
        {
            let new_start = milliseconds(playhead.min(row.end - 0.05));
            app.session
                .edit_project(|project| project.trim_clip_start(row.clip_id, new_start));
        }
    });
    ui.horizontal(|ui| {
        let mut end = row.end;
        if ui.add(time_field(&mut end)).changed() {
            let new_end = milliseconds(end.max(row.start + 0.05));
            app.session
                .edit_project(|project| project.trim_clip_end(row.clip_id, new_end));
        }
        if ui
            .small_button("⏷")
            .on_hover_text("End at the playhead")
            .clicked()
        {
            let new_end = milliseconds(playhead.max(row.start + 0.05));
            app.session
                .edit_project(|project| project.trim_clip_end(row.clip_id, new_end));
        }
    });

    let mut layer = row.track_id;
    let name = layers
        .iter()
        .find(|(id, _)| *id == layer)
        .map_or("", |(_, name)| name.as_str());
    egui::ComboBox::from_id_salt(("subtitle_line_layer", row.clip_id))
        .selected_text(name)
        .width(110.0)
        .show_ui(ui, |ui| {
            for (id, name) in layers {
                ui.selectable_value(&mut layer, *id, name);
            }
        })
        .response
        .on_hover_text("The layer (singer, speaker) this line belongs to");
    if layer != row.track_id {
        app.session
            .edit_project(|project| project.move_clip_to_track(row.clip_id, layer));
    }

    let mut words = row.text.replace('\n', " | ");
    let field = ui.add(
        egui::TextEdit::singleline(&mut words)
            .desired_width(f32::INFINITY)
            .min_size(egui::vec2(220.0, 0.0)),
    );
    if field.gained_focus() {
        app.subtitle_editor.selected = Some(row.clip_id);
    }
    if field.changed() {
        let words = words.replace(" | ", "\n").replace('|', "\n");
        app.session
            .edit_project(|project| project.set_clip_text(row.clip_id, &words));
    }
}

/// Cuts a line in two at `at` seconds; the words are shared out by where
/// the cut falls in the line's time.
fn split_line(app: &mut VizxApp, row: &Row, at: f64) {
    let words: Vec<&str> = row.text.split_whitespace().collect();
    let share = ((at - row.start) / (row.end - row.start)).clamp(0.0, 1.0);
    let cut = ((words.len() as f64 * share).round() as usize).clamp(
        usize::from(words.len() > 1),
        words.len().saturating_sub(1).max(1),
    );
    let (first, second) = (
        words[..cut.min(words.len())].join(" "),
        words[cut.min(words.len())..].join(" "),
    );
    let clip = row.clip_id;
    let mut new_clip = None;
    app.session.edit_project(|project| {
        new_clip = project.split_clip(clip, milliseconds(at));
        if let Some(second_clip) = new_clip {
            project.set_clip_text(clip, &first);
            // A split copies the line; the copy gets its own words.
            project.set_clip_text(
                second_clip,
                if second.is_empty() { &first } else { &second },
            );
        }
        new_clip.is_some()
    });
    app.subtitle_editor.selected = new_clip.or(Some(clip));
}

fn save_srt(app: &mut VizxApp, rows: &[Row]) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Subtitles", &["srt"])
        .set_file_name(format!("{}.srt", app.session.project_name()))
        .save_file()
    else {
        return;
    };
    let mut text = String::new();
    for (index, row) in rows.iter().enumerate() {
        text.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            index + 1,
            srt_time(row.start),
            srt_time(row.end),
            row.text
        ));
    }
    app.subtitle_editor.message = match std::fs::write(&path, text) {
        Ok(()) => format!("Saved {} lines to {}", rows.len(), path.display()),
        Err(error) => format!("Could not save {}: {error}", path.display()),
    };
}

/// `01:02:03,456`, as .srt files write times.
fn srt_time(seconds: f64) -> String {
    let total = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02},{:03}",
        total / 3_600_000,
        total / 60_000 % 60,
        total / 1000 % 60,
        total % 1000
    )
}

fn time_field(value: &mut f64) -> egui::DragValue<'_> {
    egui::DragValue::new(value)
        .range(0.0..=36_000.0)
        .speed(0.01)
        .max_decimals(2)
        .suffix(" s")
}

fn milliseconds(seconds: f64) -> u64 {
    (seconds.max(0.0) * 1000.0).round() as u64
}

/// The look of a subtitle layer, in the Properties panel.
pub fn style_editor(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    requested_command: &mut Option<AppCommand>,
    track: &Track,
) {
    let Some(mut style) = track.subtitle_style.clone() else {
        return;
    };
    let before = style.clone();
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Subtitles").strong());
        if ui.button("Subtitle Editor...").clicked() {
            *requested_command = Some(AppCommand::OpenSubtitleEditor);
        }
    });
    ui.separator();
    ui.label(
        egui::RichText::new(format!(
            "{} lines. Their look and place are set here for all of them.",
            track.clips.len()
        ))
        .small()
        .color(theme::TEXT_MUTED),
    );

    egui::Grid::new(("subtitle_style", track.id))
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Size");
            let (smallest, largest) = SubtitleStyle::SIZE_RANGE;
            let mut percent = style.size * 100.0;
            ui.add(
                egui::Slider::new(&mut percent, smallest * 100.0..=largest * 100.0).suffix(" %"),
            );
            style.size = percent / 100.0;
            ui.end_row();

            ui.label("Color");
            color_button(ui, &mut style.color);
            ui.end_row();

            ui.label("Outline");
            ui.horizontal(|ui| {
                let mut percent = style.outline * 100.0;
                ui.add(
                    egui::Slider::new(&mut percent, 0.0..=SubtitleStyle::OUTLINE_MAX * 100.0)
                        .suffix(" %"),
                )
                .on_hover_text("Thickness, as a share of the letter size; keeps text readable");
                style.outline = percent / 100.0;
                color_button(ui, &mut style.outline_color);
            });
            ui.end_row();

            ui.label("Align");
            ui.horizontal(|ui| {
                for choice in TextAlignment::ALL {
                    ui.selectable_value(&mut style.alignment, choice, choice.label());
                }
            });
            ui.end_row();

            ui.label("Font");
            font_chooser(ui, &mut style.font_path);
            ui.end_row();

            ui.label("Fit");
            egui::ComboBox::from_id_salt(("subtitle_fit", track.id))
                .selected_text(style.fit.label())
                .show_ui(ui, |ui| {
                    for fit in SubtitleFit::ALL {
                        ui.selectable_value(&mut style.fit, fit, fit.label());
                    }
                })
                .response
                .on_hover_text("What happens to a line too wide for the frame");
            ui.end_row();

            ui.label("Margin");
            let mut percent = style.margin * 100.0;
            ui.add(
                egui::Slider::new(&mut percent, 0.0..=SubtitleStyle::MARGIN_MAX * 100.0)
                    .suffix(" %"),
            )
            .on_hover_text("Lines keep this far from the edges (share of the frame height)");
            style.margin = percent / 100.0;
            ui.end_row();

            ui.label("Fade");
            ui.add(
                egui::DragValue::new(&mut style.fade_seconds)
                    .range(0.0..=5.0)
                    .speed(0.01)
                    .max_decimals(2)
                    .suffix(" s"),
            );
            ui.end_row();

            ui.label("Pop in");
            ui.checkbox(&mut style.pop, "")
                .on_hover_text("Lines grow into their size while they fade in");
            ui.end_row();
        });

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Place").strong());
    position_editor(ui, &mut style);
    ui.label(
        egui::RichText::new(
            "Lines shown at the same time (also from other subtitle layers) are moved apart \
             automatically.",
        )
        .small()
        .color(theme::TEXT_MUTED),
    );

    if style != before {
        session.edit_project(|project| project.set_subtitle_style(track.id, style));
    }
}

fn position_editor(ui: &mut egui::Ui, style: &mut SubtitleStyle) {
    let mut is_random = matches!(style.position, SubtitlePosition::Random { .. });
    ui.horizontal(|ui| {
        let fixed = ui.selectable_value(&mut is_random, false, "Fixed");
        let random = ui
            .selectable_value(&mut is_random, true, "Random")
            .on_hover_text("Every line at another spot of an area");
        if fixed.changed() || random.changed() {
            style.position = if is_random {
                SubtitlePosition::ANYWHERE
            } else {
                SubtitlePosition::default()
            };
        }
    });
    match &mut style.position {
        SubtitlePosition::Fixed { x, y } => {
            ui.horizontal(|ui| {
                for (label, height) in [("Top", 0.14), ("Middle", 0.5), ("Bottom", 0.86)] {
                    if ui
                        .selectable_label((*y - height).abs() < 1e-6, label)
                        .clicked()
                    {
                        *y = height;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Across");
                ui.add(fraction_field(x));
                ui.label("Down");
                ui.add(fraction_field(y));
            });
        }
        SubtitlePosition::Random { area } => {
            ui.horizontal_wrapped(|ui| {
                for (label, preset) in [
                    ("Anywhere", Placement::FULL),
                    (
                        "Upper half",
                        Placement {
                            height: 0.5,
                            ..Placement::FULL
                        },
                    ),
                    (
                        "Lower half",
                        Placement {
                            y: 0.5,
                            height: 0.5,
                            ..Placement::FULL
                        },
                    ),
                    (
                        "Middle",
                        Placement {
                            x: 0.2,
                            y: 0.25,
                            width: 0.6,
                            height: 0.5,
                        },
                    ),
                ] {
                    if ui.selectable_label(*area == preset, label).clicked() {
                        *area = preset;
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label("Area x");
                ui.add(fraction_field(&mut area.x));
                ui.label("y");
                ui.add(fraction_field(&mut area.y));
                ui.label("w");
                ui.add(fraction_field(&mut area.width));
                ui.label("h");
                ui.add(fraction_field(&mut area.height));
            });
            if ui
                .button("New Random Places")
                .on_hover_text("Pick other spots for every line")
                .clicked()
            {
                style.seed = style.seed.wrapping_add(1);
            }
        }
    }
}

fn fraction_field(value: &mut f64) -> egui::DragValue<'_> {
    egui::DragValue::new(value)
        .range(0.0..=1.0)
        .speed(0.005)
        .max_decimals(2)
}

fn color_button(ui: &mut egui::Ui, color: &mut [f64; 3]) {
    let mut shown = color.map(|part| part as f32);
    if ui.color_edit_button_rgb(&mut shown).changed() {
        *color = shown.map(f64::from);
    }
}

fn font_chooser(ui: &mut egui::Ui, font_path: &mut String) {
    ui.horizontal(|ui| {
        let name = if font_path.is_empty() {
            "Built-in"
        } else {
            font_path.rsplit(['/', '\\']).next().unwrap_or(font_path)
        };
        ui.label(name.to_owned()).on_hover_text(font_path.as_str());
        if ui.button("Choose...").clicked()
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("Font", &["ttf", "otf", "ttc"])
                .pick_file()
        {
            *font_path = path.to_string_lossy().into_owned();
        }
        if !font_path.is_empty() && ui.button("Built-in").clicked() {
            font_path.clear();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srt_times_are_written_with_a_comma() {
        assert_eq!(srt_time(0.0), "00:00:00,000");
        assert_eq!(srt_time(3725.5), "01:02:05,500");
    }
}
