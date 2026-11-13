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
use vizx_core::{EditorSession, Selection};
use vizx_project::{TrackId, TrackKind};

use crate::theme;

const RULER_HEIGHT: f32 = 22.0;
const TRACK_HEIGHT: f32 = 28.0;
const TRACK_HEADER_WIDTH: f32 = 150.0;
/// Upper zoom limit, in pixels per second of project time.
const MAX_PIXELS_PER_SECOND: f32 = 400.0;
/// Labelled ruler marks are kept at least this far apart.
const MIN_LABEL_SPACING: f32 = 70.0;
/// Width kept free in "fit" mode so a vertical scroll bar does not force a
/// horizontal one.
const SCROLL_BAR_ALLOWANCE: f32 = 14.0;
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
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
enum Zoom {
    /// The whole project length fills the panel width, whatever that is.
    #[default]
    FitProject,
    /// A fixed scale chosen by the user.
    PixelsPerSecond(f32),
}

pub fn show(ui: &mut egui::Ui, session: &mut EditorSession, view: &mut TimelineView) {
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

    let scroll_offset = show_lanes(ui, session, view, &scale, lanes_rect);
    view.scroll_offset = scroll_offset;

    show_ruler(ui, &scale, ruler_rect, scroll_offset.x);
    show_track_headers(ui, session, headers_rect, scroll_offset.y);
    show_corner(ui, view, corner_rect);

    let separator = Stroke::new(1.0, theme::SEPARATOR);
    let painter = ui.painter_at(panel_rect);
    painter.vline(split_x, panel_rect.y_range(), separator);
    painter.hline(panel_rect.x_range(), split_y, separator);

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
    background.context_menu(|ui| add_track_menu(ui, session));

    let mut scroll_area = egui::ScrollArea::both()
        .id_salt("timeline_lanes")
        .auto_shrink([false, false]);
    if let Some(offset) = view.pending_horizontal_scroll.take() {
        scroll_area = scroll_area.horizontal_scroll_offset(offset);
    }

    let mut lanes_ui = ui.new_child(egui::UiBuilder::new().max_rect(lanes_rect));
    let output = scroll_area.show_viewport(&mut lanes_ui, |ui, visible| {
        let track_count = session.project().tracks().len();
        let content_size = egui::vec2(scale.content_width(), track_count as f32 * TRACK_HEIGHT);
        let (content_rect, response) = ui.allocate_exact_size(content_size, Sense::click());
        let painter = ui.painter();
        let separator = Stroke::new(1.0, theme::SEPARATOR);
        let selection = session.selection();

        for (index, track) in session.project().tracks().iter().enumerate() {
            let top = content_rect.top() + index as f32 * TRACK_HEIGHT;
            let lane_rect = Rect::from_min_size(
                egui::pos2(content_rect.left(), top),
                egui::vec2(content_rect.width(), TRACK_HEIGHT),
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

        // Clicking a lane selects its track.
        if response.clicked()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let index = ((pointer.y - content_rect.top()) / TRACK_HEIGHT).floor() as usize;
            let clicked_track = session.project().tracks().get(index).map(|track| track.id);
            session.select(clicked_track.map_or(Selection::Nothing, Selection::Track));
        }
        response.context_menu(|ui| add_track_menu(ui, session));
    });

    output.state.offset
}

fn show_ruler(ui: &egui::Ui, scale: &TimeScale, ruler_rect: Rect, scroll_x: f32) {
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
}

fn show_track_headers(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
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
    let mut kind_to_add: Option<TrackKind> = None;
    let selection = session.selection();

    for (index, track) in session.project().tracks().iter().enumerate() {
        let top = headers_rect.top() - scroll_y + index as f32 * TRACK_HEIGHT;
        let row_rect = Rect::from_min_size(
            egui::pos2(headers_rect.left(), top),
            egui::vec2(headers_rect.width(), TRACK_HEIGHT),
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
            Rect::from_min_size(row_rect.left_top(), egui::vec2(3.0, TRACK_HEIGHT)),
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
            if ui.button("Delete Track").clicked() {
                track_to_remove = Some(track.id);
            }
            ui.separator();
            kind_to_add = kind_to_add.or(add_track_buttons(ui));
        });
    }

    // The empty space under the last header also offers "add track".
    let empty_top =
        headers_rect.top() - scroll_y + session.project().tracks().len() as f32 * TRACK_HEIGHT;
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
        response.context_menu(|ui| kind_to_add = kind_to_add.or(add_track_buttons(ui)));
    }

    if let Some(id) = clicked_track {
        session.select(Selection::Track(id));
    }
    if let Some(id) = track_to_remove {
        session.remove_track(id);
    }
    if let Some(kind) = kind_to_add {
        session.add_track(kind);
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

/// Context menu content for adding a track directly through the session.
fn add_track_menu(ui: &mut egui::Ui, session: &mut EditorSession) {
    if let Some(kind) = add_track_buttons(ui) {
        session.add_track(kind);
    }
}

/// One "Add ..." button per track kind. Returns the kind clicked.
fn add_track_buttons(ui: &mut egui::Ui) -> Option<TrackKind> {
    let mut clicked_kind = None;
    for kind in TrackKind::ALL {
        if ui.button(format!("Add {}", kind.label())).clicked() {
            clicked_kind = Some(kind);
        }
    }
    clicked_kind
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
    fn time_labels_are_formatted_as_minutes_and_seconds() {
        assert_eq!(format_time(0.0, false), "0:00");
        assert_eq!(format_time(65.0, false), "1:05");
        assert_eq!(format_time(65.5, true), "1:05.5");
        assert_eq!(format_time(599.96, true), "10:00.0");
    }
}
