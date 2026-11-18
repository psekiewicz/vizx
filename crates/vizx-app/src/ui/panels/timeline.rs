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

use std::collections::HashMap;

use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Rect, Sense, Stroke};
use vizx_audio::WaveformPeaks;
use vizx_core::{AudioState, AudioStore, EditorSession, Selection, VisualizerCatalog};
use vizx_project::{
    ClipId, EntranceExit, LayerMove, MediaId, Reveal, RevealKind, Track, TrackId, TrackKind,
};

use crate::app::AppCommand;
use crate::theme;
use crate::thumbnails::Thumbnails;
use crate::ui::{add_menu_section, add_submenu};

const RULER_HEIGHT: f32 = 22.0;
const VISUAL_TRACK_HEIGHT: f32 = 28.0;
/// Audio tracks are taller, to leave room for the waveform.
const AUDIO_TRACK_HEIGHT: f32 = 64.0;
const TRACK_HEADER_WIDTH: f32 = 150.0;
/// Color of beats placed by hand.
const MANUAL_BEAT_COLOR: Color32 = Color32::from_rgb(90, 230, 255);
/// Height of the strip along the bottom of an audio clip where beats are
/// placed by hand, in screen points.
const BEAT_STRIP_HEIGHT: f32 = 14.0;
/// A click closer than this to a hand-placed beat removes it, in screen
/// points.
const BEAT_REMOVE_REACH: f32 = 5.0;
/// Half the size of the diamond that marks a keyframe, in screen points.
const KEYFRAME_MARK_SIZE: f32 = 4.5;
/// How much one click on a zoom button zooms.
const ZOOM_BUTTON_FACTOR: f32 = 1.5;
/// Width of the zones at a clip's left and right edge that trim it when
/// dragged, in screen points.
const CLIP_EDGE_WIDTH: f32 = 7.0;
/// A dragged clip edge closer than this to a snap point jumps onto it, in
/// screen points.
const CLIP_SNAP_DISTANCE: f32 = 7.0;
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
    thumbnails: &mut Thumbnails,
    catalog: &mut VisualizerCatalog,
    requested_command: &mut Option<AppCommand>,
) {
    // Visualizer plugins are labelled with the name from their manifest,
    // not with their folder name.
    let plugin_names: HashMap<MediaId, String> = session
        .project()
        .media()
        .iter()
        .filter(|source| source.is_visualizer_plugin())
        .filter_map(|source| {
            let manifest = catalog.plugin(&source.path).manifest.as_ref().ok()?;
            Some((source.id, manifest.name.clone()))
        })
        .collect();
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
        (audio_store, thumbnails, &plugin_names),
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
    show_corner(ui, view, &scale, corner_rect);

    let separator = Stroke::new(1.0, theme::SEPARATOR);
    let painter = ui.painter_at(panel_rect);
    painter.vline(split_x, panel_rect.y_range(), separator);
    painter.hline(panel_rect.x_range(), split_y, separator);

    // The playhead: a line across the ruler and all lanes, drawn last so it
    // is on top of everything.
    let timeline_rect = ruler_rect.union(lanes_rect);

    // While looping, the loop's edges run down through all lanes.
    if let Some((start, end)) = session.loop_region().filter(|_| session.is_looping()) {
        let edge_painter = ui.painter_at(lanes_rect);
        for seconds in [start, end] {
            let x = lanes_rect.left() - scroll_offset.x + seconds as f32 * scale.pixels_per_second;
            edge_painter.vline(
                x,
                lanes_rect.y_range(),
                Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.6)),
            );
        }
    }
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
    // What clips show: waveforms, thumbnails and plugin names.
    (audio_store, thumbnails, plugin_names): (
        &AudioStore,
        &mut Thumbnails,
        &HashMap<MediaId, String>,
    ),
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
        let (content_rect, response) =
            ui.allocate_exact_size(content_size, Sense::click_and_drag());
        let painter = ui.painter();
        let separator = Stroke::new(1.0, theme::SEPARATOR);
        let selection = session.selection();

        for (track, row) in session.project().tracks().iter().zip(&rows.rows) {
            let lane_rect = Rect::from_min_size(
                egui::pos2(content_rect.left(), content_rect.top() + row.top),
                egui::vec2(content_rect.width(), row.height),
            );
            if session
                .selected_track()
                .is_some_and(|selected| selected.id == track.id)
            {
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
        let playhead_milliseconds = (session.playhead_seconds() * 1000.0).round() as u64;
        let milliseconds_at =
            |x: f32| ((x - content_rect.left()) / scale.pixels_per_second * 1000.0).max(0.0) as u64;

        // Moments a dragged clip edge jumps onto when it comes close: the
        // start of the project, the playhead and the edges of other clips.
        let snap_distance = (CLIP_SNAP_DISTANCE / scale.pixels_per_second * 1000.0).round() as u64;
        let mut snap_points: Vec<(Option<ClipId>, u64)> =
            vec![(None, 0), (None, playhead_milliseconds)];
        for clip in session
            .project()
            .tracks()
            .iter()
            .flat_map(|track| &track.clips)
        {
            snap_points.push((Some(clip.id), clip.start_milliseconds));
            snap_points.push((Some(clip.id), clip.end_milliseconds()));
        }

        // A clip drag in progress, remembered between frames. Unlike in the
        // preview, the project is changed while dragging, so the waveform
        // and everything else follow the mouse.
        let drag_id = ui.id().with("timeline_clip_drag");
        let mut drag: Option<ClipDrag> = ui.data(|data| data.get_temp(drag_id));
        // Collected in the loop (which borrows the project) and applied
        // after it.
        let mut clicked_clip: Option<(ClipId, ClipClick)> = None;
        // Where each clip is drawn, for selecting with a rectangle.
        let mut clip_rects: Vec<(ClipId, Rect)> = Vec::new();
        let mut edit: Option<(ClipId, ClipGrab, u64)> = None;
        // The track a moved clip was dragged onto, if it is another one.
        let mut new_track: Option<(ClipId, TrackId)> = None;

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
                let is_selected = session.is_clip_selected(track.id, clip.id);
                clip_rects.push((clip.id, clip_rect));
                let color = track_color(track.kind);
                let fill_strength = if is_selected { 0.55 } else { 0.35 };
                painter.rect_filled(clip_rect, 2.0, color.gamma_multiply(fill_strength));
                painter.rect_stroke(
                    clip_rect,
                    2.0,
                    if selection == Selection::Clip(clip.id) {
                        Stroke::new(1.5, Color32::WHITE)
                    } else {
                        Stroke::new(1.0, color)
                    },
                    egui::StrokeKind::Inside,
                );

                // Nothing drawn for a clip may spill outside of it.
                let clip_painter = painter.with_clip_rect(clip_rect.intersect(painter.clip_rect()));
                let source = session.project().media_source(clip.media);

                // Pictures in a row along the clip, like a film strip.
                if let Some((texture, aspect)) =
                    source.and_then(|source| thumbnails.get(ui.ctx(), source))
                {
                    let inner = clip_rect.shrink(2.0);
                    let tile_width = (inner.height() * aspect).max(4.0);
                    let first = ((visible_left - inner.left()) / tile_width)
                        .floor()
                        .max(0.0) as usize;
                    let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                    for index in first.. {
                        let x = inner.left() + index as f32 * tile_width;
                        if x > inner.right().min(visible_right) {
                            break;
                        }
                        let tile = Rect::from_min_size(
                            egui::pos2(x, inner.top()),
                            egui::vec2(tile_width, inner.height()),
                        );
                        clip_painter.image(texture, tile, uv, Color32::from_white_alpha(170));
                    }
                }
                let waveform = source.and_then(|source| audio_store.state(&source.path));

                let status = match waveform {
                    Some(AudioState::Ready(loaded)) => {
                        draw_waveform(
                            &clip_painter,
                            clip_rect,
                            visible_left..=visible_right,
                            &loaded.peaks,
                            // A trimmed clip shows its own part of the file.
                            clip.source_offset_milliseconds as f64 / 1000.0,
                            scale.pixels_per_second,
                            color,
                        );
                        // Beats drawn by hand are shown instead of the
                        // found ones, in their own color.
                        let manual = source.and_then(|source| source.manual_beats.as_ref());
                        let beats: Vec<vizx_audio::Beat> = match manual {
                            Some(manual) => manual
                                .iter()
                                .map(|seconds| vizx_audio::Beat {
                                    seconds: *seconds,
                                    strength: 1.0,
                                })
                                .collect(),
                            None => loaded
                                .beats_at(source.and_then(|source| source.tempo_bpm))
                                .0
                                .as_ref()
                                .clone(),
                        };
                        draw_beat_marks(
                            &clip_painter,
                            clip_rect,
                            visible_left..=visible_right,
                            &beats,
                            clip.source_offset_milliseconds as f64 / 1000.0,
                            scale.pixels_per_second,
                            manual.is_some(),
                        );
                        ""
                    }
                    Some(AudioState::Loading) => "  (reading waveform...)",
                    Some(AudioState::Failed(_)) => "  (waveform unavailable)",
                    None => "",
                };

                // The label stays readable at the left edge of the view when
                // the clip starts off-screen.
                let file_name = match source {
                    Some(source) => plugin_names
                        .get(&source.id)
                        .map_or(source.display_name(), String::as_str),
                    None => "Missing media",
                };
                // On a dark backing, so it stays readable over thumbnails.
                let label = clip_painter.layout_no_wrap(
                    format!("{file_name}{status}"),
                    FontId::proportional(11.0),
                    Color32::WHITE,
                );
                let label_position =
                    egui::pos2(left.max(visible_left) + 6.0, clip_rect.top() + 3.0);
                clip_painter.rect_filled(
                    Rect::from_min_size(
                        label_position - egui::vec2(3.0, 1.0),
                        label.size() + egui::vec2(6.0, 2.0),
                    ),
                    2.0,
                    Color32::from_black_alpha(140),
                );
                clip_painter.galley(label_position, label, Color32::WHITE);

                // While a clip is dragged, show exactly where it is.
                if drag.is_some_and(|drag| drag.clip == clip.id) {
                    let times = format!(
                        "{} - {}",
                        format_time(clip.start_milliseconds as f32 / 1000.0, true),
                        format_time(clip.end_milliseconds() as f32 / 1000.0, true)
                    );
                    let position =
                        egui::pos2(left.max(visible_left) + 6.0, clip_rect.bottom() - 3.0);
                    let galley =
                        painter.layout_no_wrap(times, FontId::monospace(10.0), Color32::WHITE);
                    let background = Rect::from_min_size(
                        position - egui::vec2(3.0, galley.size().y + 1.0),
                        galley.size() + egui::vec2(6.0, 2.0),
                    );
                    painter.rect_filled(background, 2.0, Color32::from_black_alpha(190));
                    painter.galley(
                        position - egui::vec2(0.0, galley.size().y),
                        galley,
                        Color32::WHITE,
                    );
                }

                // The clip itself: click to select, drag to move. Its two
                // edges come afterwards, so they are "on top" of it: drag
                // them to trim. Very narrow clips have no edge zones, or
                // there would be no room left to grab the clip itself.
                let edge_width = if width >= 4.0 * CLIP_EDGE_WIDTH {
                    CLIP_EDGE_WIDTH
                } else {
                    0.0
                };
                let grab_zones = [
                    (ClipGrab::Move, clip_rect),
                    (
                        ClipGrab::TrimStart,
                        Rect::from_min_size(
                            clip_rect.left_top(),
                            egui::vec2(edge_width, clip_rect.height()),
                        ),
                    ),
                    (
                        ClipGrab::TrimEnd,
                        Rect::from_min_size(
                            egui::pos2(clip_rect.right() - edge_width, clip_rect.top()),
                            egui::vec2(edge_width, clip_rect.height()),
                        ),
                    ),
                ];
                for (grab, zone) in grab_zones {
                    if zone.width() <= 0.0 {
                        continue;
                    }
                    let response = ui.interact(
                        zone,
                        ui.id().with(("timeline_clip", clip.id, grab)),
                        if grab == ClipGrab::Move {
                            Sense::click_and_drag()
                        } else {
                            Sense::drag()
                        },
                    );
                    let response = response.on_hover_cursor(if grab == ClipGrab::Move {
                        CursorIcon::Grab
                    } else {
                        CursorIcon::ResizeHorizontal
                    });
                    // Shift+click adds to the selection.
                    let with_shift = ui.input(|input| input.modifiers.shift);
                    if response.clicked() {
                        clicked_clip = Some((
                            clip.id,
                            if with_shift {
                                ClipClick::Toggle
                            } else {
                                ClipClick::Only
                            },
                        ));
                    } else if response.secondary_clicked() || response.drag_started() {
                        clicked_clip = Some((clip.id, ClipClick::Keep));
                    }

                    // The edge that follows the mouse: the right one when
                    // trimming the end, otherwise the left one.
                    let edge = if grab == ClipGrab::TrimEnd {
                        clip.end_milliseconds()
                    } else {
                        clip.start_milliseconds
                    };
                    if response.drag_started()
                        && let Some(pointer) = response.interact_pointer_pos()
                    {
                        drag = Some(ClipDrag {
                            clip: clip.id,
                            grab,
                            // Keeps the grabbed spot under the mouse.
                            grab_offset: milliseconds_at(pointer.x) as i64 - edge as i64,
                        });
                    }
                    let is_this_drag =
                        drag.is_some_and(|drag| drag.clip == clip.id && drag.grab == grab);
                    if response.dragged()
                        && is_this_drag
                        && let (Some(active), Some(pointer)) =
                            (drag, response.interact_pointer_pos())
                    {
                        let wanted =
                            (milliseconds_at(pointer.x) as i64 - active.grab_offset).max(0) as u64;
                        let target = snapped_edge(
                            wanted,
                            // A moved clip can also snap with its far end.
                            (grab == ClipGrab::Move).then_some(clip.duration_milliseconds),
                            clip.id,
                            &snap_points,
                            snap_distance,
                        );
                        edit = Some((clip.id, grab, target));

                        // Dragged up or down onto another track of the
                        // same kind: the clip changes tracks.
                        let track_under_pointer = rows
                            .index_at(pointer.y - content_rect.top())
                            .and_then(|index| session.project().tracks().get(index));
                        if grab == ClipGrab::Move
                            && let Some(target_track) = track_under_pointer
                            && target_track.id != track.id
                            && target_track.kind == track.kind
                        {
                            new_track = Some((clip.id, target_track.id));
                        }
                    }
                    if response.drag_stopped() && is_this_drag {
                        drag = None;
                    }

                    if grab == ClipGrab::Move {
                        let can_split = playhead_milliseconds
                            >= clip.start_milliseconds
                                + vizx_project::Clip::MIN_DURATION_MILLISECONDS
                            && playhead_milliseconds
                                + vizx_project::Clip::MIN_DURATION_MILLISECONDS
                                <= clip.end_milliseconds();
                        response.context_menu(|ui| {
                            ui.label(egui::RichText::new(file_name).strong());
                            ui.separator();
                            let split =
                                egui::Button::new("Split at Playhead").shortcut_text("Ctrl+K");
                            if ui.add_enabled(can_split, split).clicked() {
                                *requested_command = Some(AppCommand::SplitClip);
                            }
                            let duplicate = egui::Button::new("Duplicate").shortcut_text("Ctrl+D");
                            if ui.add(duplicate).clicked() {
                                *requested_command = Some(AppCommand::DuplicateClip);
                            }
                            let delete = egui::Button::new("Delete Clip").shortcut_text("Del");
                            if ui.add(delete).clicked() {
                                *requested_command = Some(AppCommand::DeleteSelection);
                            }
                            if track.kind == TrackKind::Audio
                                && !track.muted
                                && ui
                                    .button("Separate Stems")
                                    .on_hover_text(
                                        "Split the song into vocals, drums, bass and other, \
                                         each on a muted track that visualizers can follow",
                                    )
                                    .clicked()
                            {
                                *requested_command = Some(AppCommand::SeparateStems);
                            }
                            ui.separator();
                            add_submenu(ui, requested_command);
                        });
                    }
                }
            }
        }

        // Keyframes: a small diamond on the clip for each one inside it,
        // on top of everything else. Clicking one moves the playhead there.
        let mut jump_to_keyframe: Option<(ClipId, f64)> = None;
        for (track, row) in session.project().tracks().iter().zip(&rows.rows) {
            let middle_y = content_rect.top() + row.top + row.height - CLIP_VERTICAL_MARGIN - 6.0;
            for clip in &track.clips {
                for keyframe in &clip.keyframes {
                    if !(0..=clip.duration_milliseconds as i64)
                        .contains(&keyframe.time_milliseconds)
                    {
                        continue; // trimmed away
                    }
                    let seconds = (clip.start_milliseconds as i64 + keyframe.time_milliseconds)
                        as f64
                        / 1000.0;
                    let x = content_rect.left() + seconds as f32 * scale.pixels_per_second;
                    if x < visible_left - 6.0 || x > visible_right + 6.0 {
                        continue;
                    }
                    let center = egui::pos2(x, middle_y);
                    let diamond = vec![
                        center + egui::vec2(0.0, -KEYFRAME_MARK_SIZE),
                        center + egui::vec2(KEYFRAME_MARK_SIZE, 0.0),
                        center + egui::vec2(0.0, KEYFRAME_MARK_SIZE),
                        center + egui::vec2(-KEYFRAME_MARK_SIZE, 0.0),
                    ];
                    painter.add(egui::Shape::convex_polygon(
                        diamond,
                        Color32::from_rgb(255, 210, 90),
                        Stroke::new(1.0, Color32::BLACK),
                    ));
                    let response = ui
                        .interact(
                            Rect::from_center_size(
                                center,
                                egui::Vec2::splat(2.0 * KEYFRAME_MARK_SIZE + 2.0),
                            ),
                            ui.id().with((
                                "timeline_keyframe",
                                clip.id,
                                keyframe.time_milliseconds,
                            )),
                            Sense::click(),
                        )
                        .on_hover_text(format!(
                            "Keyframe at {} ({})",
                            format_time(seconds as f32, true),
                            keyframe.easing.label()
                        ));
                    if response.clicked() {
                        jump_to_keyframe = Some((clip.id, seconds));
                    }
                }
            }
        }

        // Entrance and exit: a shaded ramp at each end of a visual clip, as
        // long as the animation lasts. The selected clip gets a round
        // handle on each ramp: drag it to make the animation longer or
        // shorter (out of nothing it starts as a fade), right-click it to
        // choose the kind.
        let mut reveal_edit: Option<(ClipId, EntranceExit)> = None;
        for (track, row) in session.project().tracks().iter().zip(&rows.rows) {
            if !track.kind.is_layer() {
                continue;
            }
            let top = content_rect.top() + row.top + CLIP_VERTICAL_MARGIN;
            let bottom = content_rect.top() + row.top + row.height - CLIP_VERTICAL_MARGIN;
            for clip in &track.clips {
                let left = content_rect.left()
                    + clip.start_milliseconds as f32 / 1000.0 * scale.pixels_per_second;
                let width = clip.duration_milliseconds as f32 / 1000.0 * scale.pixels_per_second;
                let right = left + width;
                if right < visible_left || left > visible_right {
                    continue;
                }
                let clip_seconds = clip.duration_milliseconds as f64 / 1000.0;
                let (entrance, exit) = clip.entrance_exit.lengths(clip_seconds);
                let ramp_color = Color32::from_rgba_unmultiplied(255, 255, 255, 60);
                let line = Stroke::new(1.0, Color32::from_white_alpha(200));
                let entrance_x = left + entrance as f32 * scale.pixels_per_second;
                let exit_x = right - exit as f32 * scale.pixels_per_second;
                if entrance > 0.0 {
                    // Rising from nothing at the start to full at its end.
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(left, bottom),
                            egui::pos2(entrance_x, top),
                            egui::pos2(left, top),
                        ],
                        ramp_color,
                        Stroke::NONE,
                    ));
                    painter.line_segment(
                        [egui::pos2(left, bottom), egui::pos2(entrance_x, top)],
                        line,
                    );
                }
                if exit > 0.0 {
                    painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(exit_x, top),
                            egui::pos2(right, bottom),
                            egui::pos2(right, top),
                        ],
                        ramp_color,
                        Stroke::NONE,
                    ));
                    painter
                        .line_segment([egui::pos2(exit_x, top), egui::pos2(right, bottom)], line);
                }

                if selection != Selection::Clip(clip.id) || width < 4.0 * CLIP_EDGE_WIDTH {
                    continue;
                }
                // Handles sit just inside the trim zones when there is no
                // animation yet, so both can be grabbed.
                let handle_inset = CLIP_EDGE_WIDTH + 4.0;
                let entrance_handle = egui::pos2(entrance_x.max(left + handle_inset), top + 4.0);
                let exit_handle = egui::pos2(exit_x.min(right - handle_inset), top + 4.0);
                for (is_entrance, position) in [(true, entrance_handle), (false, exit_handle)] {
                    let reveal = if is_entrance {
                        clip.entrance_exit.entrance
                    } else {
                        clip.entrance_exit.exit
                    };
                    painter.circle(
                        position,
                        4.0,
                        if reveal.is_none() {
                            Color32::from_gray(60)
                        } else {
                            Color32::WHITE
                        },
                        Stroke::new(1.0, Color32::BLACK),
                    );
                    let name = if is_entrance { "Entrance" } else { "Exit" };
                    let tip = if reveal.is_none() {
                        format!("{name}: none\nDrag to add a fade\nRight-click: choose the kind")
                    } else {
                        format!(
                            "{name}: {} {:.2} s\nDrag: length\nRight-click: choose the kind",
                            reveal.kind.label(),
                            reveal.seconds
                        )
                    };
                    let response = ui
                        .interact(
                            Rect::from_center_size(position, egui::Vec2::splat(12.0)),
                            ui.id().with(("timeline_reveal", clip.id, is_entrance)),
                            Sense::click_and_drag(),
                        )
                        .on_hover_cursor(CursorIcon::ResizeHorizontal)
                        .on_hover_text(tip);
                    let mut changed = reveal;
                    if response.dragged()
                        && let Some(pointer) = response.interact_pointer_pos()
                    {
                        let pixels = if is_entrance {
                            pointer.x - left
                        } else {
                            right - pointer.x
                        };
                        let seconds = f64::from(pixels.max(0.0) / scale.pixels_per_second);
                        if seconds < Reveal::SECONDS_RANGE.0 {
                            changed.kind = RevealKind::None;
                        } else {
                            if changed.is_none() {
                                changed.kind = RevealKind::Fade;
                            }
                            changed.seconds = seconds.min(clip_seconds);
                        }
                    }
                    response.context_menu(|ui| {
                        ui.label(egui::RichText::new(name).strong());
                        ui.separator();
                        for kind in RevealKind::ALL {
                            if ui
                                .selectable_label(changed.kind == kind, kind.label())
                                .clicked()
                            {
                                changed.kind = kind;
                                ui.close();
                            }
                        }
                    });
                    if changed != reveal {
                        let mut value = clip.entrance_exit;
                        if is_entrance {
                            value.entrance = changed;
                        } else {
                            value.exit = changed;
                        }
                        reveal_edit = Some((clip.id, value));
                    }
                }
            }
        }
        if let Some((clip_id, value)) = reveal_edit {
            session.edit_project(|project| project.set_clip_entrance_exit(clip_id, value));
        }

        // Placing beats by hand: on the selected audio clip whose file uses
        // hand-placed beats, the strip along its bottom takes clicks.
        let mut beat_edit: Option<(MediaId, f64, bool)> = None;
        // A hand-placed beat being dragged: (file, beat number, new time).
        let beat_drag_id = ui.id().with("beat_drag");
        let mut beat_move: Option<(MediaId, usize, f64)> = None;
        if let Some(selected) = session.selected_clip_id()
            && let Some((track, clip)) = session.project().clip(selected)
            && let Some(source) = session.project().media_source(clip.media)
            && let Some(manual) = &source.manual_beats
            && let Some(index) = session
                .project()
                .tracks()
                .iter()
                .position(|other| other.id == track.id)
        {
            let row = &rows.rows[index];
            let left = content_rect.left()
                + clip.start_milliseconds as f32 / 1000.0 * scale.pixels_per_second;
            let width = clip.duration_milliseconds as f32 / 1000.0 * scale.pixels_per_second;
            let bottom = content_rect.top() + row.top + row.height - CLIP_VERTICAL_MARGIN;
            let strip = Rect::from_min_max(
                egui::pos2(left, bottom - BEAT_STRIP_HEIGHT),
                egui::pos2(left + width, bottom),
            );
            painter.rect_filled(strip, 0.0, MANUAL_BEAT_COLOR.gamma_multiply(0.15));
            let response = ui
                .interact(
                    strip,
                    ui.id().with(("beat_strip", selected)),
                    Sense::click_and_drag(),
                )
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text("Click: add beat\nClick a beat: remove it\nDrag a beat: move it");
            let offset = clip.source_offset_milliseconds as f64 / 1000.0;
            let reach = f64::from(BEAT_REMOVE_REACH / scale.pixels_per_second);
            let seconds_at = |x: f32| offset + f64::from((x - left) / scale.pixels_per_second);
            if response.clicked()
                && let Some(pointer) = response.interact_pointer_pos()
            {
                let seconds_in_file = seconds_at(pointer.x);
                let is_on_a_beat = manual
                    .iter()
                    .any(|beat| (beat - seconds_in_file).abs() <= reach);
                beat_edit = Some((source.id, seconds_in_file, is_on_a_beat));
            }
            if response.drag_started()
                && let Some(pointer) = response.interact_pointer_pos()
            {
                let seconds_in_file = seconds_at(pointer.x);
                let nearest = (0..manual.len())
                    .filter(|index| (manual[*index] - seconds_in_file).abs() <= reach)
                    .min_by(|a, b| {
                        (manual[*a] - seconds_in_file)
                            .abs()
                            .total_cmp(&(manual[*b] - seconds_in_file).abs())
                    });
                if let Some(index) = nearest {
                    ui.data_mut(|data| data.insert_temp(beat_drag_id, index));
                }
            }
            let dragged_beat: Option<usize> = ui.data(|data| data.get_temp(beat_drag_id));
            if let Some(index) = dragged_beat
                && response.dragged()
                && let Some(pointer) = response.interact_pointer_pos()
            {
                beat_move = Some((source.id, index, seconds_at(pointer.x)));
            }
            if response.drag_stopped() {
                ui.data_mut(|data| data.remove::<usize>(beat_drag_id));
            }
        }

        ui.data_mut(|data| match drag {
            Some(drag) => {
                data.insert_temp(drag_id, drag);
            }
            None => data.remove::<ClipDrag>(drag_id),
        });

        // Clicking a lane selects its track; clicking a clip selects it.
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
        if let Some((media_id, seconds, is_on_a_beat)) = beat_edit {
            let reach = f64::from(BEAT_REMOVE_REACH / scale.pixels_per_second);
            session.edit_project(|project| {
                if is_on_a_beat {
                    project.remove_manual_beat_near(media_id, seconds, reach)
                } else {
                    project.add_manual_beat(media_id, seconds)
                }
            });
        }
        if let Some((media_id, index, seconds)) = beat_move {
            session.edit_project(|project| project.move_manual_beat(media_id, index, seconds));
        }
        if let Some((clip_id, seconds)) = jump_to_keyframe {
            session.select(Selection::Clip(clip_id));
            session.set_playhead_seconds(seconds);
        }
        if let Some((clip_id, click)) = clicked_clip {
            match click {
                ClipClick::Only => session.select(Selection::Clip(clip_id)),
                ClipClick::Toggle => session.toggle_clip_selection(clip_id),
                // Grabbing a clip of the selection keeps the others with it.
                ClipClick::Keep => {
                    if !session.selected_clip_ids().contains(&clip_id) {
                        session.select(Selection::Clip(clip_id));
                    }
                }
            }
        }

        // Dragging over empty lanes draws a rectangle; the clips it touches
        // become selected.
        let marquee_id = ui.id().with("timeline_marquee");
        if response.drag_started()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            ui.data_mut(|data| data.insert_temp(marquee_id, pointer));
        }
        let marquee_start: Option<egui::Pos2> = ui.data(|data| data.get_temp(marquee_id));
        if let (Some(start), Some(pointer)) = (marquee_start, response.interact_pointer_pos())
            && response.dragged()
        {
            let area = Rect::from_two_pos(start, pointer);
            ui.painter().rect(
                area,
                0.0,
                theme::ACCENT.gamma_multiply(0.12),
                Stroke::new(1.0, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
            let touched: Vec<ClipId> = clip_rects
                .iter()
                .filter(|(_, rect)| rect.intersects(area))
                .map(|(id, _)| *id)
                .collect();
            session.select_clips(&touched);
        }
        if response.drag_stopped() {
            ui.data_mut(|data| data.remove::<egui::Pos2>(marquee_id));
        }
        if let Some((clip_id, track_id)) = new_track {
            let moves_group = session.selected_clip_ids().len() > 1
                && session.selected_clip_ids().contains(&clip_id);
            if moves_group {
                session.move_selected_clips_to_track(clip_id, track_id);
            } else {
                session.move_clip_to_track(clip_id, track_id);
            }
        }
        if let Some((clip_id, grab, target)) = edit {
            let moves_group = grab == ClipGrab::Move
                && session.selected_clip_ids().len() > 1
                && session.selected_clip_ids().contains(&clip_id);
            if moves_group {
                session.move_selected_clips(clip_id, target);
            } else {
                match grab {
                    ClipGrab::Move => session.move_clip(clip_id, target),
                    ClipGrab::TrimStart => session.trim_clip_start(clip_id, target),
                    ClipGrab::TrimEnd => session.trim_clip_end(clip_id, target),
                };
            }
        }
    });
    output.state.offset
}

/// How a click on a clip changes the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipClick {
    /// A plain click: only this clip is selected.
    Only,
    /// Shift+click: this clip joins the selection or leaves it.
    Toggle,
    /// A right-click or the start of a drag: a clip of the selection keeps
    /// the others selected, any other clip becomes the only one.
    Keep,
}

/// What part of a clip the mouse took hold of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ClipGrab {
    /// The clip itself: dragging moves it along the timeline.
    Move,
    /// Its left edge: dragging changes where it starts.
    TrimStart,
    /// Its right edge: dragging changes where it ends.
    TrimEnd,
}

/// A clip being dragged on the timeline.
#[derive(Debug, Clone, Copy)]
struct ClipDrag {
    clip: ClipId,
    grab: ClipGrab,
    /// Distance in milliseconds from the dragged edge to the spot the mouse
    /// took hold of, so that spot stays under the mouse.
    grab_offset: i64,
}

/// Where a dragged edge ends up: at `wanted`, or on a snap point if one is
/// within `snap_distance`.
///
/// `far_end_distance` is given when a whole clip is moved (its length):
/// then its far end may snap as well, and the result is still where the
/// dragged (near) edge goes. Snap points that belong to the dragged clip
/// itself are ignored.
fn snapped_edge(
    wanted: u64,
    far_end_distance: Option<u64>,
    dragged_clip: ClipId,
    snap_points: &[(Option<ClipId>, u64)],
    snap_distance: u64,
) -> u64 {
    let nearest = |edge: u64| {
        snap_points
            .iter()
            .filter(|(owner, _)| *owner != Some(dragged_clip))
            .map(|(_, point)| *point)
            .filter(|point| point.abs_diff(edge) <= snap_distance)
            .min_by_key(|point| point.abs_diff(edge))
    };

    if let Some(point) = nearest(wanted) {
        return point;
    }
    if let Some(length) = far_end_distance
        && let Some(point) = nearest(wanted + length)
    {
        return point.saturating_sub(length);
    }
    wanted
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
    let seconds_at =
        |x: f32| f64::from((x - ruler_rect.left() + scroll_x) / scale.pixels_per_second);
    let x_at =
        |seconds: f64| ruler_rect.left() - scroll_x + seconds as f32 * scale.pixels_per_second;

    // Shift+drag marks the loop region; a plain click or drag scrubs.
    let loop_anchor_id = ui.id().with("loop_anchor");
    if response.drag_started()
        && ui.input(|input| input.modifiers.shift)
        && let Some(pointer) = response.interact_pointer_pos()
    {
        ui.data_mut(|data| data.insert_temp(loop_anchor_id, seconds_at(pointer.x)));
    }
    let loop_anchor: Option<f64> = ui.data(|data| data.get_temp(loop_anchor_id));
    let is_marking_loop = loop_anchor.is_some() && response.dragged();
    if let (Some(anchor), Some(pointer)) = (loop_anchor, response.interact_pointer_pos())
        && response.dragged()
    {
        session.set_loop_region(anchor, seconds_at(pointer.x));
    }
    if response.drag_stopped() {
        ui.data_mut(|data| data.remove::<f64>(loop_anchor_id));
    }

    let is_scrubbing = !is_marking_loop && (response.clicked() || response.dragged());
    if is_scrubbing && let Some(pointer) = response.interact_pointer_pos() {
        session.set_playhead_seconds(seconds_at(pointer.x));
    }
    response.context_menu(|ui| {
        let has_loop = session.loop_region().is_some();
        if ui
            .button("Loop Start at Playhead")
            .on_hover_text("I")
            .clicked()
        {
            session.set_loop_start_at_playhead();
            ui.close();
        }
        if ui
            .button("Loop End at Playhead")
            .on_hover_text("O")
            .clicked()
        {
            session.set_loop_end_at_playhead();
            ui.close();
        }
        if ui
            .add_enabled(has_loop, egui::Button::new("Clear Loop"))
            .clicked()
        {
            session.clear_loop();
            ui.close();
        }
        ui.separator();
        if ui
            .button("Add Marker at Playhead")
            .on_hover_text("M")
            .clicked()
        {
            session.add_marker_at_playhead();
            ui.close();
        }
    });

    let painter = ui.painter_at(ruler_rect);
    painter.rect_filled(ruler_rect, 0.0, theme::PANEL_BACKGROUND);

    // The loop region: bright while looping, faint while not.
    if let Some((start, end)) = session.loop_region() {
        let band = Rect::from_x_y_ranges(x_at(start)..=x_at(end), ruler_rect.y_range());
        let color = if session.is_looping() {
            theme::ACCENT.gamma_multiply(0.35)
        } else {
            Color32::from_gray(90).gamma_multiply(0.35)
        };
        painter.rect_filled(band, 0.0, color);
    }

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

    show_markers(ui, session, ruler_rect, &x_at, &seconds_at);
    is_scrubbing
}

/// The markers on the ruler: a flag with the marker's name. A click moves
/// the playhead there, a drag moves the marker, a right-click renames or
/// deletes it.
fn show_markers(
    ui: &egui::Ui,
    session: &mut EditorSession,
    ruler_rect: Rect,
    x_at: &dyn Fn(f64) -> f32,
    seconds_at: &dyn Fn(f32) -> f64,
) {
    const MARKER_COLOR: Color32 = Color32::from_rgb(240, 190, 60);
    let painter = ui.painter_at(ruler_rect);
    let markers = session.project().markers().to_vec();
    for (index, marker) in markers.iter().enumerate() {
        let seconds = marker.milliseconds as f64 / 1000.0;
        let x = x_at(seconds);
        if x < ruler_rect.left() - 120.0 || x > ruler_rect.right() {
            continue;
        }
        let flag = [
            egui::pos2(x, ruler_rect.top() + 2.0),
            egui::pos2(x + 8.0, ruler_rect.top() + 6.0),
            egui::pos2(x, ruler_rect.top() + 10.0),
        ];
        painter.add(egui::Shape::convex_polygon(
            flag.to_vec(),
            MARKER_COLOR,
            Stroke::NONE,
        ));
        painter.vline(x, ruler_rect.y_range(), Stroke::new(1.0, MARKER_COLOR));
        let label = painter.text(
            egui::pos2(x + 10.0, ruler_rect.top() + 1.0),
            Align2::LEFT_TOP,
            &marker.name,
            FontId::proportional(11.0),
            MARKER_COLOR,
        );

        let hit = Rect::from_min_max(
            egui::pos2(x - 4.0, ruler_rect.top()),
            egui::pos2(label.right().max(x + 10.0), ruler_rect.top() + 12.0),
        );
        let response = ui
            .interact(
                hit,
                ui.id().with(("marker", index)),
                Sense::click_and_drag(),
            )
            .on_hover_text(format!(
                "{} (drag to move, right-click to rename)",
                marker.name
            ));
        if response.clicked() {
            session.set_playhead_seconds(seconds);
        }
        if response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let milliseconds = (seconds_at(pointer.x).max(0.0) * 1000.0).round() as u64;
            session.edit_project(|project| {
                project.move_marker(index, milliseconds);
                true
            });
        }
        response.context_menu(|ui| {
            let buffer_id = ui.id().with(("marker_name", index));
            let mut name: String = ui
                .data(|data| data.get_temp(buffer_id))
                .unwrap_or_else(|| marker.name.clone());
            let field = ui.text_edit_singleline(&mut name);
            if field.changed() {
                ui.data_mut(|data| data.insert_temp(buffer_id, name.clone()));
                session.edit_project(|project| project.rename_marker(index, &name));
            }
            ui.separator();
            ui.label("Transition into this scene");
            let mut transition = marker.transition;
            for kind in vizx_project::TransitionKind::ALL {
                ui.radio_value(&mut transition.kind, kind, kind.label());
            }
            if transition != marker.transition {
                session.edit_project(|project| project.set_marker_transition(index, transition));
            }
            ui.separator();
            if ui.button("Delete Marker").clicked() {
                session.edit_project(|project| project.remove_marker(index));
                ui.close();
            }
        });
    }
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
    let mut layer_to_convert: Option<TrackId> = None;
    let selected_track = session.selected_track().map(|track| track.id);
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

        let is_selected = selected_track == Some(track.id);
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
            if track.muted {
                format!("{} (muted)", track.name)
            } else {
                track.name.clone()
            },
            FontId::proportional(12.0),
            if is_selected {
                Color32::WHITE
            } else if track.muted {
                theme::TEXT_MUTED
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
            if track.kind.is_layer() {
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
            if track.kind == TrackKind::Subtitles {
                if ui.button("Subtitle Editor...").clicked() {
                    clicked_track = Some(track.id);
                    *requested_command = Some(AppCommand::OpenSubtitleEditor);
                }
                ui.separator();
            } else if track.kind == TrackKind::Visual && holds_only_texts(session.project(), track)
            {
                let button = ui.button("Convert to Subtitle Layer").on_hover_text(
                    "The texts become lines of subtitles: one look for all of them, kept \
                     inside the frame, and edited in the Subtitle Editor",
                );
                if button.clicked() {
                    layer_to_convert = Some(track.id);
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
    if let Some(id) = layer_to_convert {
        session.edit_project(|project| project.convert_to_subtitles(id));
    }
}

/// Whether a layer holds texts and nothing else (lyrics added as texts).
fn holds_only_texts(project: &vizx_project::Project, track: &Track) -> bool {
    !track.clips.is_empty()
        && track.clips.iter().all(|clip| {
            project
                .media_source(clip.media)
                .is_some_and(|media| media.text().is_some())
        })
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
                TrackKind::Visual | TrackKind::Subtitles => VISUAL_TRACK_HEIGHT,
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
    source_offset_seconds: f64,
    pixels_per_second: f32,
    color: Color32,
) {
    let stroke = Stroke::new(1.0, color);
    let center_y = clip_rect.center().y;
    let half_height = (clip_rect.height() / 2.0 - 2.0).max(1.0);
    let last_x = clip_rect.right().min(*visible_x.end());

    let mut x = clip_rect.left().max(*visible_x.start()).floor();
    while x < last_x {
        // Time within the file: the clip's own time plus what was trimmed
        // off its start.
        let start_seconds =
            source_offset_seconds + f64::from((x - clip_rect.left()) / pixels_per_second);
        let end_seconds =
            source_offset_seconds + f64::from((x + 1.0 - clip_rect.left()) / pixels_per_second);
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

/// Small marks along the bottom edge of an audio clip where its beats are.
/// Stronger beats get taller marks.
fn draw_beat_marks(
    painter: &egui::Painter,
    clip_rect: Rect,
    visible_x: std::ops::RangeInclusive<f32>,
    beats: &[vizx_audio::Beat],
    source_offset_seconds: f64,
    pixels_per_second: f32,
    is_drawn_by_hand: bool,
) {
    // Too crowded to be useful when zoomed far out (hand-placed beats are
    // usually fewer, and the user is working on them).
    let seconds_per_pixel = 1.0 / f64::from(pixels_per_second);
    let limit = if is_drawn_by_hand { 0.25 } else { 0.08 };
    if seconds_per_pixel > limit {
        return;
    }
    let stroke = if is_drawn_by_hand {
        Stroke::new(1.5, MANUAL_BEAT_COLOR)
    } else {
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 220, 120, 200))
    };
    for beat in beats {
        let seconds_in_clip = beat.seconds - source_offset_seconds;
        if seconds_in_clip < 0.0 {
            continue;
        }
        let x = clip_rect.left() + seconds_in_clip as f32 * pixels_per_second;
        if x > clip_rect.right() || x > *visible_x.end() {
            break;
        }
        if x < *visible_x.start() {
            continue;
        }
        let height = 3.0 + 6.0 * beat.strength;
        painter.vline(
            x,
            (clip_rect.bottom() - height)..=clip_rect.bottom(),
            stroke,
        );
    }
}

/// Top-left corner of the panel: zoom status and the "fit" button.
fn show_corner(ui: &mut egui::Ui, view: &mut TimelineView, scale: &TimeScale, corner_rect: Rect) {
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
        .on_hover_text("Zoom to fit (Ctrl+wheel to zoom)");
    if response.clicked() {
        view.zoom = Zoom::FitProject;
        view.pending_horizontal_scroll = Some(0.0);
    }

    // Zoom out / in, for those without a mouse wheel at hand.
    for (index, (label, factor, tooltip)) in [
        ("\u{2212}", 1.0 / ZOOM_BUTTON_FACTOR, "Zoom out"),
        ("+", ZOOM_BUTTON_FACTOR, "Zoom in"),
    ]
    .into_iter()
    .enumerate()
    {
        let rect = Rect::from_min_size(
            button_rect.right_top() + egui::vec2(4.0 + index as f32 * 26.0, 0.0),
            egui::vec2(22.0, button_rect.height()),
        );
        let button = egui::Button::new(egui::RichText::new(label).small());
        if ui.put(rect, button).on_hover_text(tooltip).clicked() {
            let (zoom, scroll) = zoomed(scale, view.scroll_offset.x, factor);
            view.zoom = zoom;
            view.pending_horizontal_scroll = Some(scroll);
        }
    }
}

/// The zoom and horizontal scroll position after zooming by `factor`,
/// keeping the left edge of the view at the same moment of the project.
fn zoomed(scale: &TimeScale, scroll_x: f32, factor: f32) -> (Zoom, f32) {
    let pixels_per_second = (scale.pixels_per_second * factor)
        .clamp(scale.fit_pixels_per_second, MAX_PIXELS_PER_SECOND);
    // Zooming all the way out returns to "fit" mode.
    let zoom = if pixels_per_second <= scale.fit_pixels_per_second {
        Zoom::FitProject
    } else {
        Zoom::PixelsPerSecond(pixels_per_second)
    };
    let seconds_at_left_edge = scroll_x / scale.pixels_per_second;
    (zoom, seconds_at_left_edge * pixels_per_second)
}

fn track_color(kind: TrackKind) -> Color32 {
    match kind {
        TrackKind::Audio => Color32::from_rgb(86, 148, 222),
        TrackKind::Visual => theme::ACCENT,
        TrackKind::Subtitles => Color32::from_rgb(232, 196, 92),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_buttons_keep_the_left_edge_and_end_in_fit_mode() {
        // 100 s shown in 1000 px: 10 px per second when fitted.
        let fitted = scale(Zoom::FitProject, 100.0, 1000.0 + SCROLL_BAR_ALLOWANCE);

        let (zoom, scroll) = zoomed(&fitted, 0.0, 2.0);
        assert_eq!(zoom, Zoom::PixelsPerSecond(20.0));
        assert_eq!(scroll, 0.0);

        // Zoomed in and scrolled to second 30: it stays at the left edge.
        let closer = scale(
            Zoom::PixelsPerSecond(20.0),
            100.0,
            1000.0 + SCROLL_BAR_ALLOWANCE,
        );
        let (zoom, scroll) = zoomed(&closer, 600.0, 2.0);
        assert_eq!(zoom, Zoom::PixelsPerSecond(40.0));
        assert_eq!(scroll, 1200.0);

        // Zooming out past the whole project goes back to fitting it.
        assert_eq!(zoomed(&closer, 600.0, 0.25).0, Zoom::FitProject);
        // And there is an upper limit.
        let (zoom, _) = zoomed(&closer, 0.0, 1000.0);
        assert_eq!(zoom, Zoom::PixelsPerSecond(MAX_PIXELS_PER_SECOND));
    }

    #[test]
    fn a_dragged_edge_snaps_to_nearby_points_but_not_to_its_own_clip() {
        let mut project = vizx_project::Project::with_starter_tracks();
        project.settings.duration_milliseconds = 10_000;
        let (_, layer) = project.add_milkdrop_preset("a.milk");
        let dragged = project.track(layer).unwrap().clips[0].id;
        let (_, other_layer) = project.add_milkdrop_preset("b.milk");
        let other = project.track(other_layer).unwrap().clips[0].id;
        // The start of the project, the playhead at 5 s, another clip from
        // 2 s to 3 s, and the dragged clip's own edge at 7 s.
        let points = [
            (None, 0),
            (None, 5_000),
            (Some(other), 2_000),
            (Some(other), 3_000),
            (Some(dragged), 7_000),
        ];
        let snap = |wanted, length| snapped_edge(wanted, length, dragged, &points, 100);

        assert_eq!(snap(4_950, None), 5_000);
        assert_eq!(snap(4_800, None), 4_800); // too far away
        assert_eq!(snap(2_060, None), 2_000);
        assert_eq!(snap(7_010, None), 7_010); // its own edge does not count
        // The nearest of two candidates wins.
        assert_eq!(snapped_edge(2_480, None, dragged, &points, 600), 2_000);

        // Moving a 1 s clip: its end reaches the playhead first.
        assert_eq!(snap(4_040, Some(1_000)), 4_000);
        // Its start snaps in preference to its end.
        assert_eq!(snap(2_950, Some(2_000)), 3_000);
    }
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
