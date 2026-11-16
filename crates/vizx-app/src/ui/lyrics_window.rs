//! The Lyrics window: the quick way to make a "text video". The words of a
//! song are pasted in, one subtitle per line, and turned into text clips on
//! a "Lyrics" layer, timed by tapping along with the music, spread over a
//! stretch of time, one every few beats, or read from an .lrc/.srt file.

use eframe::egui;
use vizx_core::{LyricLine, LyricsStyle};
use vizx_project::{ClipId, TextAlignment, TextProperties, TrackId, TrackKind};

use crate::app::{AppCommand, VizxApp};
use crate::theme;

/// How the pasted lines get their times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Timing {
    /// Played back, a key starts each line.
    Tap,
    /// Spread over a stretch of time, longer lines longer.
    Spread,
    /// One line every few beats.
    Beats,
}

/// Lines being tapped along with the music.
struct Tapping {
    track_id: TrackId,
    lines: Vec<String>,
    /// The line the next tap starts.
    next_line: usize,
    /// The line shown right now, until the next tap ends it.
    open_clip: Option<ClipId>,
}

/// Everything the Lyrics window remembers while the app runs.
pub struct LyricsWindow {
    pub is_open: bool,
    text: String,
    style: LyricsStyle,
    timing: Timing,
    spread_from: f64,
    spread_to: f64,
    beats_per_line: usize,
    beats_from: f64,
    /// The layer new lines go to; `None` makes a new "Lyrics" layer.
    target: Option<TrackId>,
    tapping: Option<Tapping>,
    message: String,
}

impl Default for LyricsWindow {
    fn default() -> Self {
        Self {
            is_open: false,
            text: String::new(),
            style: LyricsStyle::default(),
            timing: Timing::Tap,
            spread_from: 0.0,
            spread_to: 0.0,
            beats_per_line: 8,
            beats_from: 0.0,
            target: None,
            tapping: None,
            message: String::new(),
        }
    }
}

impl LyricsWindow {
    /// Opens the window; the times to spread over start as the whole
    /// project.
    pub fn open(&mut self, project_seconds: f64) {
        if !self.is_open && self.spread_to <= self.spread_from {
            self.spread_to = project_seconds;
        }
        self.is_open = true;
    }
}

pub fn show(ctx: &egui::Context, app: &mut VizxApp) {
    if !app.lyrics.is_open {
        return;
    }
    handle_tapping_keys(ctx, app);

    let mut is_open = app.lyrics.is_open;
    egui::Window::new("Lyrics")
        .open(&mut is_open)
        .collapsible(false)
        .resizable(true)
        .default_width(380.0)
        .default_pos(ctx.content_rect().center() - egui::vec2(190.0, 260.0))
        .show(ctx, |ui| {
            if app.lyrics.tapping.is_some() {
                tapping_panel(ui, app);
            } else {
                setup_panel(ui, app);
            }
            let window = &app.lyrics;
            if !window.message.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(&window.message)
                        .small()
                        .color(theme::TEXT_MUTED),
                );
            }
        });
    if !is_open {
        stop_tapping(app);
    }
    app.lyrics.is_open = is_open;
}

/// The words, their look and how they are timed.
fn setup_panel(ui: &mut egui::Ui, app: &mut VizxApp) {
    ui.label("One subtitle per line. Use | for a line break.");
    egui::ScrollArea::vertical()
        .max_height(200.0)
        .show(ui, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut app.lyrics.text)
                    .desired_rows(8)
                    .desired_width(f32::INFINITY)
                    .hint_text("Paste the lyrics here"),
            );
        });

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Look").strong());
    ui.separator();
    style_editor(ui, &mut app.lyrics.style);
    let lyric_layers: Vec<(TrackId, String)> = app
        .session
        .project()
        .tracks()
        .iter()
        .filter(|track| track.kind == TrackKind::Visual)
        .map(|track| (track.id, track.name.clone()))
        .collect();
    if app
        .lyrics
        .target
        .is_some_and(|id| !lyric_layers.iter().any(|(layer, _)| *layer == id))
    {
        app.lyrics.target = None;
    }
    ui.horizontal(|ui| {
        ui.label("Layer");
        let current = app
            .lyrics
            .target
            .and_then(|id| lyric_layers.iter().find(|(layer, _)| *layer == id))
            .map_or("New \"Lyrics\" layer", |(_, name)| name.as_str());
        egui::ComboBox::from_id_salt("lyrics_layer")
            .selected_text(current)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut app.lyrics.target, None, "New \"Lyrics\" layer");
                for (id, name) in &lyric_layers {
                    ui.selectable_value(&mut app.lyrics.target, Some(*id), name);
                }
            });
        if let Some(track_id) = app.lyrics.target
            && ui
                .button("Restyle Layer")
                .on_hover_text("Apply this style to every text on the layer")
                .clicked()
        {
            app.lyrics.message = match app.session.restyle_lyrics(track_id, &app.lyrics.style) {
                Ok(count) => format!("{count} texts updated"),
                Err(problem) => format!("Could not restyle: {problem}"),
            };
        }
    });

    ui.add_space(6.0);
    ui.label(egui::RichText::new("Timing").strong());
    ui.separator();
    ui.horizontal(|ui| {
        let window = &mut app.lyrics;
        ui.selectable_value(&mut window.timing, Timing::Tap, "Tap Along")
            .on_hover_text("Press Enter at each line while the song plays");
        ui.selectable_value(&mut window.timing, Timing::Spread, "Spread")
            .on_hover_text("Spread the lines evenly over a time range");
        ui.selectable_value(&mut window.timing, Timing::Beats, "On Beats")
            .on_hover_text("One line every N beats");
    });

    let lines = vizx_core::split_lyrics(&app.lyrics.text);
    let project_seconds = app.session.project().settings.duration_seconds();
    match app.lyrics.timing {
        Timing::Tap => {
            ui.label(
                egui::RichText::new(
                    "Playback starts at the playhead. Press Enter when a line starts \
                     (it ends the one before), Backspace to end a line early, Esc to stop.",
                )
                .small()
                .color(theme::TEXT_MUTED),
            );
            if ui
                .add_enabled(!lines.is_empty(), egui::Button::new("Start Tapping"))
                .clicked()
            {
                start_tapping(app, lines);
            }
        }
        Timing::Spread => {
            ui.horizontal(|ui| {
                let window = &mut app.lyrics;
                ui.label("From");
                ui.add(seconds_field(&mut window.spread_from, project_seconds));
                ui.label("to");
                ui.add(seconds_field(&mut window.spread_to, project_seconds));
                if ui
                    .small_button("Playhead")
                    .on_hover_text("Start at playhead")
                    .clicked()
                {
                    window.spread_from = app.session.playhead_seconds();
                }
            });
            if ui
                .add_enabled(!lines.is_empty(), egui::Button::new("Add Lines"))
                .clicked()
            {
                let window = &app.lyrics;
                let timed = vizx_core::spread_lines(&lines, window.spread_from, window.spread_to);
                add_lines(app, &timed);
            }
        }
        Timing::Beats => {
            ui.horizontal(|ui| {
                let window = &mut app.lyrics;
                ui.label("Every");
                ui.add(egui::DragValue::new(&mut window.beats_per_line).range(1..=64));
                ui.label("beats, from");
                ui.add(seconds_field(&mut window.beats_from, project_seconds));
                if ui.small_button("Playhead").clicked() {
                    window.beats_from = app.session.playhead_seconds();
                }
            });
            if ui
                .add_enabled(!lines.is_empty(), egui::Button::new("Add Lines"))
                .clicked()
            {
                let beats = app.mix_beat_seconds();
                let window = &app.lyrics;
                let timed = vizx_core::lines_on_beats(
                    &lines,
                    &beats,
                    window.beats_from,
                    window.beats_per_line,
                );
                if timed.is_empty() {
                    app.lyrics.message =
                        "No beats in this range (no audio, or still loading)".to_owned();
                } else {
                    add_lines(app, &timed);
                }
            }
        }
    }

    ui.add_space(6.0);
    if ui
        .button("Import .lrc or .srt File...")
        .on_hover_text("Timed lyrics (LRC) or subtitles (SRT)")
        .clicked()
    {
        app.request(AppCommand::ImportLyricsFile);
    }
}

/// The look shared by all lines.
fn style_editor(ui: &mut egui::Ui, style: &mut LyricsStyle) {
    egui::Grid::new("lyrics_style")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Size");
            let (smallest, _) = TextProperties::SIZE_RANGE;
            let mut percent = style.text.size * 100.0;
            ui.add(egui::Slider::new(&mut percent, smallest * 100.0..=30.0).suffix(" %"));
            style.text.size = percent / 100.0;
            ui.end_row();

            ui.label("Color");
            let mut color = style.text.color.map(|part| part as f32);
            if ui.color_edit_button_rgb(&mut color).changed() {
                style.text.color = color.map(f64::from);
            }
            ui.end_row();

            ui.label("Align");
            ui.horizontal(|ui| {
                for choice in TextAlignment::ALL {
                    ui.selectable_value(&mut style.text.alignment, choice, choice.label());
                }
            });
            ui.end_row();

            ui.label("Font");
            ui.horizontal(|ui| {
                let name = if style.text.font_path.is_empty() {
                    "Built-in"
                } else {
                    style
                        .text
                        .font_path
                        .rsplit(['/', '\\'])
                        .next()
                        .unwrap_or(&style.text.font_path)
                };
                ui.label(name).on_hover_text(&style.text.font_path);
                if ui.button("Choose...").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("Font", &["ttf", "otf", "ttc"])
                        .pick_file()
                {
                    style.text.font_path = path.to_string_lossy().into_owned();
                }
                if !style.text.font_path.is_empty() && ui.button("Built-in").clicked() {
                    style.text.font_path.clear();
                }
            });
            ui.end_row();

            ui.label("Position");
            ui.horizontal(|ui| {
                for (label, height) in [("Top", 0.15), ("Middle", 0.5), ("Bottom", 0.85)] {
                    if ui
                        .selectable_label((style.height - height).abs() < 1e-6, label)
                        .clicked()
                    {
                        style.height = height;
                    }
                }
                ui.add(
                    egui::DragValue::new(&mut style.height)
                        .range(0.0..=1.0)
                        .speed(0.01)
                        .max_decimals(2),
                )
                .on_hover_text("Vertical position (0 = top, 1 = bottom)");
            });
            ui.end_row();

            ui.label("Fade");
            ui.add(
                egui::DragValue::new(&mut style.fade_seconds)
                    .range(0.0..=2.0)
                    .speed(0.01)
                    .max_decimals(2)
                    .suffix(" s"),
            )
            .on_hover_text("Fade in / out duration");
            ui.end_row();

            ui.label("Pop in");
            ui.checkbox(&mut style.pop, "")
                .on_hover_text("Scale up while fading in");
            ui.end_row();
        });
}

fn seconds_field(value: &mut f64, longest: f64) -> egui::DragValue<'_> {
    egui::DragValue::new(value)
        .range(0.0..=longest.max(0.0))
        .speed(0.05)
        .max_decimals(2)
        .suffix(" s")
}

/// While tapping: which line comes next, and buttons for the same keys.
fn tapping_panel(ui: &mut egui::Ui, app: &mut VizxApp) {
    let Some(tapping) = &app.lyrics.tapping else {
        return;
    };
    let total = tapping.lines.len();
    let next = tapping.lines.get(tapping.next_line).cloned();
    let is_line_open = tapping.open_clip.is_some();

    ui.label(format!(
        "Line {} of {total}",
        (tapping.next_line + 1).min(total)
    ));
    match &next {
        Some(line) => {
            ui.label("Next:");
            ui.label(egui::RichText::new(line).size(18.0).strong());
        }
        None => {
            ui.label("All lines placed. Enter or Backspace ends the last one.");
        }
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button("Next Line (Enter)").clicked() {
            tap_next(app);
        }
        if ui
            .add_enabled(is_line_open, egui::Button::new("End Line (Backspace)"))
            .clicked()
        {
            tap_end(app);
        }
        if ui.button("Stop (Esc)").clicked() {
            stop_tapping(app);
        }
    });
}

fn handle_tapping_keys(ctx: &egui::Context, app: &mut VizxApp) {
    if app.lyrics.tapping.is_none() || ctx.egui_wants_keyboard_input() {
        return;
    }
    let consume = |key| ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, key));
    if consume(egui::Key::Enter) {
        tap_next(app);
    }
    if consume(egui::Key::Backspace) {
        tap_end(app);
    }
    if consume(egui::Key::Escape) {
        stop_tapping(app);
    }
}

fn start_tapping(app: &mut VizxApp, lines: Vec<String>) {
    let track_id = match app.lyrics.target {
        Some(track_id) => track_id,
        None => {
            let track_id = app.session.add_lyrics_track();
            app.lyrics.target = Some(track_id);
            track_id
        }
    };
    app.lyrics.tapping = Some(Tapping {
        track_id,
        lines,
        next_line: 0,
        open_clip: None,
    });
    app.lyrics.message = "Tapping: Enter = next line".to_owned();
    if !app.is_playing() {
        app.request(AppCommand::TogglePlayback);
    }
}

/// Ends the line shown now and starts the next one, at the playhead.
fn tap_next(app: &mut VizxApp) {
    let now = app.transport.heard_seconds(&app.session);
    let style = app.lyrics.style.clone();
    let Some(tapping) = &mut app.lyrics.tapping else {
        return;
    };
    if let Some(open) = tapping.open_clip.take() {
        app.session.end_lyric_line(open, now, &style);
    }
    let Some(line) = tapping.lines.get(tapping.next_line).cloned() else {
        stop_tapping(app);
        return;
    };
    let track_id = tapping.track_id;
    match app.session.start_lyric_line(track_id, &line, now, &style) {
        Ok(clip_id) => {
            if let Some(tapping) = &mut app.lyrics.tapping {
                tapping.open_clip = Some(clip_id);
                tapping.next_line += 1;
            }
        }
        Err(problem) => {
            app.lyrics.message = format!("Could not add the line: {problem}");
            stop_tapping(app);
        }
    }
}

/// Ends the line shown now, leaving a pause until the next tap.
fn tap_end(app: &mut VizxApp) {
    let now = app.transport.heard_seconds(&app.session);
    let style = app.lyrics.style.clone();
    let Some(tapping) = &mut app.lyrics.tapping else {
        return;
    };
    if let Some(open) = tapping.open_clip.take() {
        app.session.end_lyric_line(open, now, &style);
    }
    if tapping.next_line >= tapping.lines.len() {
        stop_tapping(app);
    }
}

/// Stops tapping. A line still shown ends at the playhead.
fn stop_tapping(app: &mut VizxApp) {
    let Some(tapping) = app.lyrics.tapping.take() else {
        return;
    };
    if let Some(open) = tapping.open_clip {
        let now = app.transport.heard_seconds(&app.session);
        let style = app.lyrics.style.clone();
        app.session.end_lyric_line(open, now, &style);
    }
    if app.is_playing() {
        app.request(AppCommand::TogglePlayback);
    }
    app.lyrics.message = format!(
        "Placed {} of {} lines",
        tapping.next_line,
        tapping.lines.len()
    );
}

fn add_lines(app: &mut VizxApp, lines: &[LyricLine]) {
    let style = app.lyrics.style.clone();
    app.lyrics.message = match app.session.add_lyrics(lines, &style, app.lyrics.target) {
        Ok(track_id) => {
            app.lyrics.target = Some(track_id);
            format!("Added {} lines", lines.len())
        }
        Err(problem) => format!("Could not add the lines: {problem}"),
    };
}

/// Adds the lines of a timed lyrics file (after the user picked it).
pub fn add_timed_lines(app: &mut VizxApp, lines: &[LyricLine]) {
    add_lines(app, lines);
}
