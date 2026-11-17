//! The Lyrics window: the quick way to make a "text video". The words of a
//! song are pasted in, one subtitle per line, and turned into lines of a
//! subtitle layer, timed by tapping along with the music, spread over a
//! stretch of time, one every few beats, or read from an .lrc/.srt file.
//! The lines are then fixed in the Subtitle Editor.

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
    /// The times written in the text, as in an .lrc file (recognized
    /// lyrics come like this).
    AsTimed,
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
    pub(crate) message: String,
    /// Index into `RECOGNITION_LANGUAGES` for recognizing the lyrics.
    recognition_language: usize,
    recognition_quality: vizx_core::RecognitionQuality,
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
            recognition_language: 0,
            recognition_quality: vizx_core::RecognitionQuality::default(),
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

    /// Shows recognized lyrics (LRC text) for checking, timed as written.
    pub fn show_recognized(&mut self, lrc: &str) {
        self.text = lrc.to_owned();
        self.timing = Timing::AsTimed;
        self.is_open = true;
    }
}

/// Recognizing the words from the song's vocals with the Whisper model.
fn recognition_panel(ui: &mut egui::Ui, app: &mut VizxApp) {
    if let Some(job) = app.lyrics_recognition() {
        let step = match job.step() {
            vizx_core::RecognitionStep::SeparatingVocals => "Separating the vocals",
            vizx_core::RecognitionStep::Listening => "Listening to the vocals",
        };
        let progress = job.progress();
        ui.horizontal(|ui| {
            ui.add(
                egui::ProgressBar::new(progress)
                    .desired_width(220.0)
                    .text(format!("{step} {:.0}%", progress * 100.0)),
            );
            if ui.button("Stop").clicked() {
                app.cancel_lyrics_recognition();
            }
        });
        return;
    }
    ui.horizontal(|ui| {
        let languages = vizx_core::RECOGNITION_LANGUAGES;
        let window = &mut app.lyrics;
        let current = languages
            .get(window.recognition_language)
            .map_or("Detect", |(_, name)| *name);
        egui::ComboBox::from_id_salt("recognition_language")
            .selected_text(current)
            .width(90.0)
            .show_ui(ui, |ui| {
                for (index, (_, name)) in languages.iter().enumerate() {
                    ui.selectable_value(&mut window.recognition_language, index, *name);
                }
            })
            .response
            .on_hover_text("Language of the song");
        egui::ComboBox::from_id_salt("recognition_quality")
            .selected_text(window.recognition_quality.label())
            .width(80.0)
            .show_ui(ui, |ui| {
                for quality in vizx_core::RecognitionQuality::ALL {
                    ui.selectable_value(&mut window.recognition_quality, quality, quality.label())
                        .on_hover_text(quality_hint(quality));
                }
            })
            .response
            .on_hover_text("Faster, or more accurate on unclear singing");
        if ui
            .button("Recognize from Vocals")
            .on_hover_text(
                "Separate the vocals from the song (or use its stems), then write down the \
                 sung words with their times. Runs on this computer and takes a few minutes.",
            )
            .clicked()
        {
            let language = languages
                .get(app.lyrics.recognition_language)
                .and_then(|(code, _)| code.map(str::to_owned));
            let ctx = ui.ctx().clone();
            let quality = app.lyrics.recognition_quality;
            app.recognize_lyrics(&ctx, language, quality);
        }
    });
}

/// What a recognition quality means, with its time measured on a 4-core
/// laptop (i7-4720HQ) for a four-minute song whose vocals were already
/// separated. Most of the time goes to listening, which all three share.
fn quality_hint(quality: vizx_core::RecognitionQuality) -> &'static str {
    match quality {
        vizx_core::RecognitionQuality::Fast => {
            "The most likely words, no second tries.\nAbout 3.4 x the song's length."
        }
        vizx_core::RecognitionQuality::Balanced => {
            "Unclear parts are tried again.\nAbout as fast as Fast; slower on very unclear singing."
        }
        vizx_core::RecognitionQuality::Accurate => {
            "Several candidate lines are weighed (beam search).\nAbout 3.5 x the song's length."
        }
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
    recognition_panel(ui, app);
    ui.add_space(4.0);
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
        .filter(|track| track.kind == TrackKind::Subtitles)
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
        ui.selectable_value(&mut window.timing, Timing::AsTimed, "As Timed")
            .on_hover_text("Use the [mm:ss.xx] times written before the lines");
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
        Timing::AsTimed => match vizx_core::parse_timed_lyrics(&app.lyrics.text) {
            Ok(timed) => {
                ui.label(
                    egui::RichText::new(format!(
                        "{} timed lines. Edit the words freely; keep the [mm:ss.xx] times.",
                        timed.len()
                    ))
                    .small()
                    .color(theme::TEXT_MUTED),
                );
                if ui.button("Add Lines").clicked() {
                    add_timed_lines(app, &timed);
                }
            }
            Err(_) => {
                ui.label(
                    egui::RichText::new(
                        "No times found. Lines need a time in front, like [01:23.45], or use \
                         Recognize from Vocals.",
                    )
                    .small()
                    .color(theme::TEXT_MUTED),
                );
            }
        },
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
