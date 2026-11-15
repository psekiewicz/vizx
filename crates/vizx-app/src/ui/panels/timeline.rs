//! The timeline panel.
//!
//! Everything shown here comes from the project: the tracks, and the length
//! of the time axis. The panel adapts to its size and to the project:
//!
//! - By default the whole project length is fitted to the panel width.
//! - Ctrl + mouse wheel zooms around the mouse cursor.
//! - When zoomed in, or when there are more tracks than fit, the lanes
//!   scroll; the ruler and the track names stay in place and follow along.
//! - Ruler marks get denser or sparser with the zoom level.

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke};
use vizx_audio::WaveformPeaks;
use vizx_core::{AudioState, AudioStore, EditorSession, Selection};
use vizx_project::{LayerMove, Track, TrackId, TrackKind};

use crate::app::AppCommand;
use crate::theme;
use crate::ui::{add_menu_section, add_submenu};

const RULER_HEIGHT: f32 = 22.0;
const VISUAL_TRACK_HEIGHT: f32 = 28.0;
/// Audio tracks are taller, to leave room for the waveform.
const AUDIO_TRACK_HEIGHT: f32 = 64.0;
const TRACK_HEADER_WIDTH: f32 = 150.0;
/// Gap between a clip and the top and bottom edges of its lane.
const CLIP_VERTICAL_MARGIN: f32 = 3.0;
/// Upper zoom limit, in pixels per second of project time.
const MAX_PIXELS_PER_SECOND: f32 = 400.0;
/// Labelled ruler marks are kept at least this far apart.
const MIN_LABEL_SPACING: f32 = 70.0;
/// Width kept free in "fit" mode so a vertical scroll bar does not force a
/// horizontal one.
const SCROLL_BAR_ALLOWANCE: f32 = 14.0;
/// Space kept to the left of the playhead when the view scrolls to follow it.
const FOLLOW_MARGIN: f32 = 40.0;
/// Candidate distances between labelled ruler marks, in seconds.
const RULER_INTERVALS_SECONDS: [f32; 14] = [
    0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1800.0,
];

/// How the timeline is currently being looked at. This is UI state: it is
/// not part of the project and is not saved.
#[derive(Debug, Clone, Copy, Default)]
pub struct TimelineView {
    zoom: Zoom,
    /// Scroll position of the lanes, remembered from the previous frame.
    scroll_offset: egui::Vec2,
    /// Horizontal scroll position to apply on the next frame, set by zooming.
    pending_horizontal_scroll: Option<f32>,
    /// Playhead position seen on the previous frame, to notice it moving.
    last_playhead_seconds: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum Zoom {
    /// The whole project length fills the panel width, whatever that is.
    #[default]
    FitProject,
    /// A fixed scale chosen by the user.
    PixelsPerSecond(f32),
}

pub fn show(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    view: &mut TimelineView,
    audio_store: &AudioStore,
    requested_command: &mut Option<AppCommand>,
) {
    let panel_rect = ui.available_rect_before_wrap();
    ui.painter_at(panel_rect)
        .rect_filled(panel_rect, 0.0, theme::CANVAS_BACKGROUND);

    // Split the panel into four regions:
    //   corner | ruler
    //   -------+------
    //   headers| lanes
    let split_x = (panel_rect.left() + TRACK_HEADER_WIDTH).min(panel_rect.right());
    let split_y = (panel_rect.top() + RULER_HEIGHT).min(panel_rect.bottom());
    let corner_rect = Rect::from_min_max(panel_rect.min, egui::pos2(split_x, split_y));
    let ruler_rect = Rect::from_min_max(
        egui::pos2(split_x, panel_rect.top()),
        egui::pos2(panel_rect.right(), split_y),
    );
    let headers_rect = Rect::from_min_max(
        egui::pos2(panel_rect.left(), split_y),
        egui::pos2(split_x, panel_rect.bottom()),
    );
    let lanes_rect = Rect::from_min_max(egui::pos2(split_x, split_y), panel_rect.max);

    let duration_seconds = session.project().settings.duration_seconds() as f32;
    let scale = TimeScale::new(view, duration_seconds, lanes_rect.width());
    handle_zoom_input(ui, view, &scale, ruler_rect.union(lanes_rect));

    let scroll_offset = show_lanes(
        ui,
        session,
        view,
        audio_store,
        requested_command,
        &scale,
        lanes_rect,
    );
    view.scroll_offset = scroll_offset;

    let is_scrubbing = show_ruler(ui, session, &scale, ruler_rect, scroll_offset.x);
    show_track_headers(
        ui,
        session,
        requested_command,
        headers_rect,
        scroll_offset.y,
    );
    show_corner(ui, view, corner_rect);

    let separator = Stroke::new(1.0, theme::SEPARATOR);
    let painter = ui.painter_at(panel_rect);
    painter.vline(split_x, panel_rect.y_range(), separator);
    painter.hline(panel_rect.x_range(), split_y, separator);

    // The playhead: a line across the ruler and all lanes, drawn last so it
    // is on top of everything.
    let timeline_rect = ruler_rect.union(lanes_rect);
    let playhead_seconds = session.playhead_seconds();
    let playhead_x =
        timeline_rect.left() - scroll_offset.x + playhead_seconds as f32 * scale.pixels_per_second;
    if timeline_rect.x_range().contains(playhead_x) {
        let playhead_painter = ui.painter_at(timeline_rect);
        playhead_painter.vline(
            playhead_x,
            timeline_rect.y_range(),
            Stroke::new(1.5, theme::PLAYHEAD),
        );
        // A small marker in the ruler makes the line easy to find.
        playhead_painter.rect_filled(
            Rect::from_center_size(
                egui::pos2(playhead_x, ruler_rect.bottom() - 4.0),
                egui::vec2(9.0, 8.0),
            ),
            1.0,
            theme::PLAYHEAD,
        );
    }

    // Follow the playhead: when it moves out of view by itself (during
    // playback, or after "go to start/end"), scroll so it is visible again.
    // Not while the user drags it, which would make the view run away.
    let has_playhead_moved = playhead_seconds != view.last_playhead_seconds;
    if has_playhead_moved && !is_scrubbing && !timeline_rect.x_range().contains(playhead_x) {
        let target = playhead_seconds as f32 * scale.pixels_per_second - FOLLOW_MARGIN;
        view.pending_horizontal_scroll = Some(target.max(0.0));
        ui.ctx().request_repaint();
    }
    view.last_playhead_seconds = playhead_seconds;

    ui.allocate_rect(panel_rect, Sense::hover());
}

/// The current mapping between project time and horizontal pixels.
struct TimeScale {
    pixels_per_second: f32,
    /// Scale at which the whole project exactly fills the lanes.
    fit_pixels_per_second: f32,
    duration_seconds: f32,
    /// Distance between labelled ruler marks at this zoom level.
    label_interval_seconds: f32,
}

impl TimeScale {
    fn new(view: &TimelineView, duration_seconds: f32, lanes_width: f32) -> Self {
        let duration_seconds = duration_seconds.max(0.001);
        let usable_width = (lanes_width - SCROLL_BAR_ALLOWANCE).max(1.0);
        let fit_pixels_per_second = (usable_width / duration_seconds).min(MAX_PIXELS_PER_SECOND);

        let pixels_per_second = match view.zoom {
            Zoom::FitProject => fit_pixels_per_second,
            // The user cannot zoom out further than the whole project.
            Zoom::PixelsPerSecond(value) => {
                value.clamp(fit_pixels_per_second, MAX_PIXELS_PER_SECOND)
            }
        };

        Self {
            pixels_per_second,
            fit_pixels_per_second,
            duration_seconds,
            label_interval_seconds: label_interval(pixels_per_second),
        }
    }

    fn content_width(&self) -> f32 {
        self.duration_seconds * self.pixels_per_second
    }
}

/// Picks the smallest "nice" interval whose marks are far enough apart to
/// be labelled without overlapping.
fn label_interval(pixels_per_second: f32) -> f32 {
    RULER_INTERVALS_SECONDS
        .into_iter()
        .find(|interval| interval * pixels_per_second >= MIN_LABEL_SPACING)
        .unwrap_or(RULER_INTERVALS_SECONDS[RULER_INTERVALS_SECONDS.len() - 1])
}

/// Formats a ruler label: `1:05`, or `1:05.5` when marks are below a second.
fn format_time(seconds: f32, show_fraction: bool) -> String {
    let total_tenths = (seconds * 10.0).round() as u32;
    let minutes = total_tenths / 600;
    let whole_seconds = (total_tenths / 10) % 60;

    if show_fraction {
        format!("{minutes}:{whole_seconds:02}.{}", total_tenths % 10)
    } else {
        format!("{minutes}:{whole_seconds:02}")
    }
}

/// Ctrl + wheel (or a pinch gesture) zooms, keeping the moment of project
/// time under the mouse cursor where it is.
fn handle_zoom_input(ui: &egui::Ui, view: &mut TimelineView, scale: &TimeScale, area: Rect) {
    let Some(pointer) = ui
        .ctx()
        .pointer_hover_pos()
        .filter(|pos| area.contains(*pos))
    else {
        return;
    };
    let zoom_factor = ui.input(|input| input.zoom_delta());
    if zoom_factor == 1.0 {
        return;
    }

    let new_pixels_per_second = (scale.pixels_per_second * zoom_factor)
        .clamp(scale.fit_pixels_per_second, MAX_PIXELS_PER_SECOND);
    let pointer_x_in_lanes = pointer.x - area.left();
    let time_under_pointer = (view.scroll_offset.x + pointer_x_in_lanes) / scale.pixels_per_second;

    // Zooming all the way out returns to "fit" mode, so the timeline goes
    // back to following the panel width.
    view.zoom = if new_pixels_per_second <= scale.fit_pixels_per_second {
        Zoom::FitProject
    } else {
        Zoom::PixelsPerSecond(new_pixels_per_second)
    };
    view.pending_horizontal_scroll =
        Some((time_under_pointer * new_pixels_per_second - pointer_x_in_lanes).max(0.0));
}

/// Draws the scrollable track lanes and returns the scroll position.
fn show_lanes(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    view: &mut TimelineView,
    audio_store: &AudioStore,
    requested_command: &mut Option<AppCommand>,
    scale: &TimeScale,
    lanes_rect: Rect,
) -> egui::Vec2 {
    // The area behind the lanes. It is registered first so the lanes, added
    // afterwards, receive clicks where they overlap it. A click on the empty
    // space below or beside the lanes clears the selection.
    let background = ui.interact(
        lanes_rect,
        ui.id().with("timeline_background"),
        Sense::click(),
    );
    if background.clicked() {
        session.select(Selection::Nothing);
    }
    background.context_menu(|ui| add_menu_section(ui, requested_command));

    let mut scroll_area = egui::ScrollArea::both()
        .id_salt("timeline_lanes")
        .auto_shrink([false, false]);
    if let Some(offset) = view.pending_horizontal_scroll.take() {
        scroll_area = scroll_area.horizontal_scroll_offset(offset);
    }

    let mut lanes_ui = ui.new_child(egui::UiBuilder::new().max_rect(lanes_rect));
    let output = scroll_area.show_viewport(&mut lanes_ui, |ui, visible| {
        let rows = TrackRows::new(session.project().tracks());
        let content_size = egui::vec2(scale.content_width(), rows.total_height);
        let (content_rect, response) = ui.allocate_exact_size(content_size, Sense::click());
        let painter = ui.painter();
        let separator = Stroke::new(1.0, theme::SEPARATOR);
        let selection = session.selection();

        for (track, row) in session.project().tracks().iter().zip(&rows.rows) {
            let lane_rect = Rect::from_min_size(
                egui::pos2(content_rect.left(), content_rect.top() + row.top),
                egui::vec2(content_rect.width(), row.height),
            );
            if selection == Selection::Track(track.id) {
                painter.rect_filled(lane_rect, 0.0, theme::ACCENT.gamma_multiply(0.12));
            }
            painter.hline(lane_rect.x_range(), lane_rect.bottom(), separator);
        }

        // Vertical guide lines under the labelled ruler marks. Only the
        // visible part is drawn, however long the project is.
        let guide = Stroke::new(1.0, theme::SEPARATOR.gamma_multiply(0.5));
        let first_mark = (visible.left() / scale.pixels_per_second / scale.label_interval_seconds)
            .floor() as u32;
        for mark in first_mark.. {
            let seconds = mark as f32 * scale.label_interval_seconds;
            let x = content_rect.left() + seconds * scale.pixels_per_second;
            if seconds > scale.duration_seconds || x > content_rect.left() + visible.right() {
                break;
            }
            painter.vline(x, content_rect.y_range(), guide);
        }

        // Clips, drawn over the guide lines.
        let visible_left = content_rect.left() + visible.left();
        let visible_right = content_rect.left() + visible.right();
        for (track, row) in session.project().tracks().iter().zip(&rows.rows) {
            let top = content_rect.top() + row.top;
            for clip in &track.clips {
                let left = content_rect.left()
                    + clip.start_milliseconds as f32 / 1000.0 * scale.pixels_per_second;
                let width = clip.duration_milliseconds as f32 / 1000.0 * scale.pixels_per_second;
                let clip_rect = Rect::from_min_size(
                    egui::pos2(left, top + CLIP_VERTICAL_MARGIN),
                    egui::vec2(width, row.height - 2.0 * CLIP_VERTICAL_MARGIN),
                );
                let color = track_color(track.kind);
                painter.rect_filled(clip_rect, 2.0, color.gamma_multiply(0.35));
                painter.rect_stroke(
                    clip_rect,
                    2.0,
                    Stroke::new(1.0, color),
                    egui::StrokeKind::Inside,
                );

                // Nothing drawn for a clip may spill outside of it.
                let clip_painter = painter.with_clip_rect(clip_rect.intersect(painter.clip_rect()));
                let source = session.project().media_source(clip.media);
                let waveform = source.and_then(|source| audio_store.state(&source.path));

                let status = match waveform {
                    Some(AudioState::Ready(loaded)) => {
                        draw_waveform(
                            &clip_painter,
                            clip_rect,
                            visible_left..=visible_right,
                            &loaded.peaks,
                            scale.pixels_per_second,
                            color,
                        );
                        ""
                    }
                    Some(AudioState::Loading) => "  (reading waveform...)",
                    Some(AudioState::Failed(_)) => "  (waveform unavailable)",
                    None => "",
                };

                // The label stays readable at the left edge of the view when
                // the clip starts off-screen.
                let file_name = source.map_or("Missing media", |source| source.display_name());
                clip_painter.text(
                    egui::pos2(left.max(visible_left) + 6.0, clip_rect.top() + 3.0),
                    Align2::LEFT_TOP,
                    format!("{file_name}{status}"),
                    FontId::proportional(11.0),
                    Color32::WHITE,
                );
            }
        }

        // Clicking a lane selects its track.
        if response.clicked()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let clicked_track = rows
                .index_at(pointer.y - content_rect.top())
                .and_then(|index| session.project().tracks().get(index))
                .map(|track| track.id);
            session.select(clicked_track.map_or(Selection::Nothing, Selection::Track));
        }
        response.context_menu(|ui| add_menu_section(ui, requested_command));
    });

    output.state.offset
}

/// Draws the time ruler. Clicking or dragging on it moves the playhead
/// ("scrubbing"). Returns whether the user is doing that right now.
fn show_ruler(
    ui: &egui::Ui,
    session: &mut EditorSession,
    scale: &TimeScale,
    ruler_rect: Rect,
    scroll_x: f32,
) -> bool {
    let response = ui.interact(
        ruler_rect,
        ui.id().with("timeline_ruler"),
        Sense::click_and_drag(),
    );
    let is_scrubbing = response.clicked() || response.dragged();
    if is_scrubbing && let Some(pointer) = response.interact_pointer_pos() {
        let seconds = (pointer.x - ruler_rect.left() + scroll_x) / scale.pixels_per_second;
        session.set_playhead_seconds(f64::from(seconds));
    }

    let painter = ui.painter_at(ruler_rect);
    painter.rect_filled(ruler_rect, 0.0, theme::PANEL_BACKGROUND);

    let stroke = Stroke::new(1.0, theme::SEPARATOR);
    // Five small marks between two labelled ones.
    let minor_interval = scale.label_interval_seconds / 5.0;
    let show_fraction = scale.label_interval_seconds < 1.0;
    let first_mark = (scroll_x / scale.pixels_per_second / minor_interval).floor() as u32;

    for mark in first_mark.. {
        let seconds = mark as f32 * minor_interval;
        let x = ruler_rect.left() - scroll_x + seconds * scale.pixels_per_second;
        if seconds > scale.duration_seconds + minor_interval * 0.5 || x > ruler_rect.right() {
            break;
        }

        let is_labelled = mark % 5 == 0;
        let mark_height = if is_labelled { 8.0 } else { 4.0 };
        painter.vline(
            x,
            (ruler_rect.bottom() - mark_height)..=ruler_rect.bottom(),
            stroke,
        );
        if is_labelled {
            painter.text(
                egui::pos2(x + 3.0, ruler_rect.top() + 2.0),
                Align2::LEFT_TOP,
                format_time(seconds, show_fraction),
                FontId::monospace(10.0),
                theme::TEXT_MUTED,
            );
        }
    }

    is_scrubbing
}

fn show_track_headers(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    requested_command: &mut Option<AppCommand>,
    headers_rect: Rect,
    scroll_y: f32,
) {
    let painter = ui.painter_at(headers_rect);
    painter.rect_filled(headers_rect, 0.0, theme::PANEL_BACKGROUND);
    let separator = Stroke::new(1.0, theme::SEPARATOR);

    // Collected first and applied after the loop, because the loop borrows
    // the track list from the session.
    let mut clicked_track: Option<TrackId> = None;
    let mut track_to_remove: Option<TrackId> = None;
    let mut layer_to_move: Option<(TrackId, LayerMove)> = None;
    let selection = session.selection();
    let rows = TrackRows::new(session.project().tracks());

    for (track, row) in session.project().tracks().iter().zip(&rows.rows) {
        let row_rect = Rect::from_min_size(
            egui::pos2(headers_rect.left(), headers_rect.top() - scroll_y + row.top),
            egui::vec2(headers_rect.width(), row.height),
        );
        let visible_rect = row_rect.intersect(headers_rect);
        if !visible_rect.is_positive() {
            continue; // scrolled out of view
        }

        let is_selected = selection == Selection::Track(track.id);
        if is_selected {
            painter.rect_filled(row_rect, 0.0, theme::ACCENT.gamma_multiply(0.25));
        }
        // Colored marker on the left edge shows the kind of track.
        painter.rect_filled(
            Rect::from_min_size(row_rect.left_top(), egui::vec2(3.0, row.height)),
            0.0,
            track_color(track.kind),
        );
        painter.text(
            row_rect.left_center() + egui::vec2(12.0, 0.0),
            Align2::LEFT_CENTER,
            &track.name,
            FontId::proportional(12.0),
            if is_selected {
                Color32::WHITE
            } else {
                ui.visuals().text_color()
            },
        );
        painter.hline(row_rect.x_range(), row_rect.bottom(), separator);

        let response = ui.interact(
            visible_rect,
            ui.id().with(("track_header", track.id)),
            Sense::click(),
        );
        if response.clicked() || response.secondary_clicked() {
            clicked_track = Some(track.id);
        }
        response.context_menu(|ui| {
            ui.label(egui::RichText::new(&track.name).strong());
            ui.separator();
            // The order of the layers decides what is in front: a layer
            // higher in the list is drawn over the ones below it.
            if track.kind == TrackKind::Visual {
                for (label, to) in [
                    ("Bring to Front", LayerMove::ToFront),
                    ("Move Up", LayerMove::Up),
                    ("Move Down", LayerMove::Down),
                    ("Send to Back", LayerMove::ToBack),
                ] {
                    if ui.button(label).clicked() {
                        layer_to_move = Some((track.id, to));
                    }
                }
                ui.separator();
            }
            if ui.button("Delete Track").clicked() {
                track_to_remove = Some(track.id);
            }
            ui.separator();
            add_submenu(ui, requested_command);
        });
    }

    // The empty space under the last header also offers the Add menu.
    let empty_top = headers_rect.top() - scroll_y + rows.total_height;
    let empty_rect = Rect::from_min_max(
        egui::pos2(headers_rect.left(), empty_top.max(headers_rect.top())),
        headers_rect.max,
    );
    if empty_rect.is_positive() {
        let response = ui.interact(
            empty_rect,
            ui.id().with("track_headers_empty"),
            Sense::click(),
        );
        if response.clicked() {
            session.select(Selection::Nothing);
        }
        response.context_menu(|ui| add_menu_section(ui, requested_command));
    }

    if let Some(id) = clicked_track {
        session.select(Selection::Track(id));
    }
    if let Some(id) = track_to_remove {
        session.remove_track(id);
    }
    if let Some((id, to)) = layer_to_move {
        session.edit_project(|project| project.move_layer(id, to));
    }
}

/// Where each track's lane is, measured down from the top of the first one.
/// Lanes have different heights, so positions cannot be computed by simply
/// multiplying an index.
struct TrackRows {
    rows: Vec<TrackRow>,
    total_height: f32,
}

struct TrackRow {
    top: f32,
    height: f32,
}

impl TrackRows {
    fn new(tracks: &[Track]) -> Self {
        let mut rows = Vec::with_capacity(tracks.len());
        let mut top = 0.0;
        for track in tracks {
            let height = match track.kind {
                TrackKind::Audio => AUDIO_TRACK_HEIGHT,
                TrackKind::Visual => VISUAL_TRACK_HEIGHT,
            };
            rows.push(TrackRow { top, height });
            top += height;
        }
        Self {
            rows,
            total_height: top,
        }
    }

    /// Index of the track whose lane contains the vertical position `y`.
    fn index_at(&self, y: f32) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| y >= row.top && y < row.top + row.height)
    }
}

/// Draws a waveform into a clip: one vertical line per pixel column, from
/// the lowest to the highest sample value within that column's slice of
/// time. Because each column asks for exactly the time it covers, the same
/// code works at every zoom level, and only visible columns are drawn.
fn draw_waveform(
    painter: &egui::Painter,
    clip_rect: Rect,
    visible_x: std::ops::RangeInclusive<f32>,
    peaks: &WaveformPeaks,
    pixels_per_second: f32,
    color: Color32,
) {
    let stroke = Stroke::new(1.0, color);
    let center_y = clip_rect.center().y;
    let half_height = (clip_rect.height() / 2.0 - 2.0).max(1.0);
    let last_x = clip_rect.right().min(*visible_x.end());

    let mut x = clip_rect.left().max(*visible_x.start()).floor();
    while x < last_x {
        let start_seconds = f64::from((x - clip_rect.left()) / pixels_per_second);
        let end_seconds = f64::from((x + 1.0 - clip_rect.left()) / pixels_per_second);
        if let Some((minimum, maximum)) = peaks.range(start_seconds, end_seconds) {
            // Positive sample values go up, which is towards smaller y.
            let top = center_y - maximum * half_height;
            let bottom = center_y - minimum * half_height;
            // Silence still gets a one-pixel line, so the clip is not blank.
            painter.vline(x + 0.5, top..=bottom.max(top + 1.0), stroke);
        }
        x += 1.0;
    }
}

/// Top-left corner of the panel: zoom status and the "fit" button.
fn show_corner(ui: &mut egui::Ui, view: &mut TimelineView, corner_rect: Rect) {
    ui.painter_at(corner_rect)
        .rect_filled(corner_rect, 0.0, theme::PANEL_BACKGROUND);

    let is_fitted = view.zoom == Zoom::FitProject;
    let button = egui::Button::new(egui::RichText::new("Fit").small()).selected(is_fitted);
    let button_rect = Rect::from_min_size(
        corner_rect.left_top() + egui::vec2(4.0, 2.0),
        egui::vec2(34.0, (corner_rect.height() - 4.0).max(0.0)),
    );
    let response = ui
        .put(button_rect, button)
        .on_hover_text("Fit the whole project to the panel width.\nCtrl + mouse wheel zooms.");
    if response.clicked() {
        view.zoom = Zoom::FitProject;
        view.pending_horizontal_scroll = Some(0.0);
    }
}

fn track_color(kind: TrackKind) -> Color32 {
    match kind {
        TrackKind::Audio => Color32::from_rgb(86, 148, 222),
        TrackKind::Visual => theme::ACCENT,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scale(zoom: Zoom, duration_seconds: f32, lanes_width: f32) -> TimeScale {
        let view = TimelineView {
            zoom,
            ..Default::default()
        };
        TimeScale::new(&view, duration_seconds, lanes_width)
    }

    #[test]
    fn fit_mode_fills_the_available_width_at_any_panel_size() {
        for lanes_width in [400.0, 1000.0, 1800.0] {
            let scale = scale(Zoom::FitProject, 180.0, lanes_width);

            let expected = lanes_width - SCROLL_BAR_ALLOWANCE;
            assert!((scale.content_width() - expected).abs() < 0.01);
        }
    }

    #[test]
    fn fit_mode_follows_the_project_duration() {
        let short = scale(Zoom::FitProject, 60.0, 1000.0);
        let long = scale(Zoom::FitProject, 600.0, 1000.0);

        assert!(short.pixels_per_second > long.pixels_per_second);
        assert!((short.content_width() - long.content_width()).abs() < 0.01);
    }

    #[test]
    fn custom_zoom_cannot_go_below_fit_or_above_the_maximum() {
        let too_small = scale(Zoom::PixelsPerSecond(0.01), 180.0, 1000.0);
        let too_large = scale(Zoom::PixelsPerSecond(100_000.0), 180.0, 1000.0);

        assert_eq!(too_small.pixels_per_second, too_small.fit_pixels_per_second);
        assert_eq!(too_large.pixels_per_second, MAX_PIXELS_PER_SECOND);
    }

    #[test]
    fn ruler_labels_get_denser_when_zooming_in() {
        let zoomed_out = label_interval(2.0);
        let zoomed_in = label_interval(200.0);

        assert!(zoomed_in < zoomed_out);
        // Labels never end up closer than the minimum spacing.
        assert!(zoomed_out * 2.0 >= MIN_LABEL_SPACING);
        assert!(zoomed_in * 200.0 >= MIN_LABEL_SPACING);
    }

    #[test]
    fn audio_tracks_are_taller_and_rows_stack_without_gaps() {
        let mut project = vizx_project::Project::new();
        project.add_track("Song", TrackKind::Audio);
        project.add_track("Layer", TrackKind::Visual);
        // Order on the timeline: the visual layer, then the audio track.
        let rows = TrackRows::new(project.tracks());

        assert_eq!(rows.rows[0].top, 0.0);
        assert_eq!(rows.rows[0].height, VISUAL_TRACK_HEIGHT);
        assert_eq!(rows.rows[1].top, VISUAL_TRACK_HEIGHT);
        assert_eq!(rows.rows[1].height, AUDIO_TRACK_HEIGHT);
        assert_eq!(rows.total_height, VISUAL_TRACK_HEIGHT + AUDIO_TRACK_HEIGHT);

        assert_eq!(rows.index_at(0.0), Some(0));
        assert_eq!(rows.index_at(VISUAL_TRACK_HEIGHT - 0.1), Some(0));
        assert_eq!(rows.index_at(VISUAL_TRACK_HEIGHT), Some(1));
        assert_eq!(rows.index_at(rows.total_height), None);
        assert_eq!(rows.index_at(-1.0), None);
    }

    #[test]
    fn time_labels_are_formatted_as_minutes_and_seconds() {
        assert_eq!(format_time(0.0, false), "0:00");
        assert_eq!(format_time(65.0, false), "1:05");
        assert_eq!(format_time(65.5, true), "1:05.5");
        assert_eq!(format_time(599.96, true), "10:00.0");
    }
}
