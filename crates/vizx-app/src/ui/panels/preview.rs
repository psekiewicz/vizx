use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Rect, Stroke, StrokeKind};
use vizx_core::{EditorSession, Selection};
use vizx_project::{ClipId, Placement, SubtitlePosition, SubtitleStyle, TrackId, TrackKind};

use crate::app::AppCommand;
use crate::theme;
use crate::visual_preview::{self, LayerState, LayerView, VisualPreview};

/// Empty space kept between the composition frame and the panel edges.
const FRAME_MARGIN: f32 = 24.0;
/// Edge length of the square resize handles, in screen points.
const HANDLE_SIZE: f32 = 8.0;
/// Distance of the rotation knob above the top edge, in screen points.
const ROTATION_HANDLE_DISTANCE: f32 = 22.0;
/// A dragged edge closer than this to a guide (frame edge or middle) jumps
/// onto it, in screen points.
const SNAP_DISTANCE: f32 = 6.0;
/// Colour of the guide lines shown while an element snaps onto them. It
/// must stand out against any picture, so it is not a theme colour.
const GUIDE_COLOR: Color32 = Color32::from_rgb(255, 64, 200);

/// Draws the composition at the playhead and lets visualizers be moved and
/// resized directly in the picture.
///
/// The frame has the shape of the project's composition size. The picture
/// in it is composed by `VisualPreview`; this panel shows it and lays the
/// editing controls over it. Clicking a clip selects its layer; the selected one gets a frame
/// with handles: drag inside to move, drag a handle to resize.
pub fn show(
    ui: &mut egui::Ui,
    session: &mut EditorSession,
    requested_command: &mut Option<AppCommand>,
    preview: &VisualPreview,
) {
    let layers = preview.layers();
    let settings = session.project().settings.clone();
    let panel_rect = ui.available_rect_before_wrap();
    let painter = ui.painter_at(panel_rect);

    painter.rect_filled(panel_rect, 0.0, theme::CANVAS_BACKGROUND);

    let composition_size = egui::vec2(settings.width as f32, settings.height as f32);
    let composition_rect = fit_inside(composition_size, panel_rect.shrink(FRAME_MARGIN));

    painter.rect_filled(composition_rect, 0.0, Color32::BLACK);
    painter.rect_stroke(
        composition_rect,
        0.0,
        Stroke::new(1.0, theme::SEPARATOR),
        StrokeKind::Outside,
    );

    // Tell the compositor how large the picture is shown, in real screen
    // pixels, and draw what it composed.
    visual_preview::set_shown_width(
        ui.ctx(),
        composition_rect.width() * ui.ctx().pixels_per_point(),
    );
    if let Some(texture) = preview.composition() {
        let whole_texture = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        painter.image(
            texture.id(),
            composition_rect,
            whole_texture,
            Color32::WHITE,
        );
    }

    // The empty area comes first, so the visualizers added after it are
    // "on top" and receive clicks before it does.
    // Dragging on it draws a selection rectangle.
    let background = ui.interact(
        panel_rect,
        ui.id().with("preview_background"),
        egui::Sense::click_and_drag(),
    );
    if background.clicked() {
        session.select(Selection::Nothing);
    }
    // A selection rectangle being drawn: where the mouse was pressed.
    let band_id = ui.id().with("preview_band");
    let mut band_start: Option<egui::Pos2> = ui.data(|data| data.get_temp(band_id));
    if background.drag_started() {
        band_start = ui.input(|input| input.pointer.press_origin());
    }
    // Shift adds to the selection instead of replacing it.
    let adds_to_selection = ui.input(|input| input.modifiers.shift);
    background.context_menu(|ui| {
        if ui.button("Project Settings...").clicked() {
            *requested_command = Some(AppCommand::OpenProjectSettings);
        }
    });

    if let Some(problem) = preview.composition_error() {
        draw_message(
            &painter,
            composition_rect,
            format!("Render error: {problem}"),
        );
    } else if layers.is_empty() {
        draw_message(
            &painter,
            composition_rect,
            "No visual clips at the playhead".to_owned(),
        );
    }

    // A move or resize in progress. It lives in egui's temporary storage
    // between frames and is written to the project when the mouse button is
    // released, so the project (and the pre-render that depends on the
    // size) changes once per drag, not once per mouse movement.
    let drag_id = ui.id().with("preview_drag");
    let mut drag: Option<Drag> = ui.data(|data| data.get_temp(drag_id));
    let mut finished_drag: Option<Drag> = None;

    // What a dragged element snaps onto: the frame's edges and middle and
    // the edges and middles of the other elements in the picture. Holding
    // Alt moves it freely.
    let is_snapping = !ui.input(|input| input.modifiers.alt);

    // The selected elements that can be moved together (lines of subtitles
    // are placed by their layer instead).
    let selected_elements: Vec<ClipId> = layers
        .iter()
        .filter(|layer| session.is_clip_selected(layer.track_id, layer.clip_id))
        .filter(|layer| {
            session
                .project()
                .track(layer.track_id)
                .is_none_or(|track| track.kind != TrackKind::Subtitles)
        })
        .map(|layer| layer.clip_id)
        .collect();
    let group_move = GroupMove::of(drag, &selected_elements);
    let group = &group_move;
    let guides = if is_snapping {
        Guides::of_picture(layers).without(&selected_elements)
    } else {
        Guides::default()
    };

    // Only the part of a visualizer inside the frame is visible.
    let layer_painter = ui.painter_at(composition_rect);
    for layer in layers {
        // The clip's frame and handles stay where its placement is, even
        // while its animation moves the picture, so they do not jump
        // around while the music plays.
        let layer_rect = to_screen(shown_placement(layer, drag, group), composition_rect);
        draw_layer_state(&layer_painter, layer_rect, &layer.state);
    }

    // The topmost element under the mouse (an effects layer only once it is
    // selected, so the layers below it stay clickable).
    let topmost_under = move |pointer: egui::Pos2, session: &EditorSession| {
        layers
            .iter()
            .rev()
            .filter(|layer| {
                !layer.shows_below || session.is_clip_selected(layer.track_id, layer.clip_id)
            })
            .find(|layer| {
                let rect = to_screen(shown_placement(layer, drag, group), composition_rect);
                is_inside_turned(rect, shown_rotation(layer, drag), pointer)
            })
            .map(|layer| (layer.track_id, layer.clip_id))
    };

    // A line of a subtitle layer is placed by its layer, so it is not an
    // element of its own: clicking it selects the whole layer, and
    // dragging it moves the layer's area (or point).
    let is_subtitle_layer = |session: &EditorSession, track_id: TrackId| {
        session
            .project()
            .track(track_id)
            .is_some_and(|track| track.kind == TrackKind::Subtitles)
    };
    let selection_for = |session: &EditorSession, (track_id, clip_id): (TrackId, ClipId)| {
        if is_subtitle_layer(session, track_id) {
            Selection::Track(track_id)
        } else {
            Selection::Clip(clip_id)
        }
    };
    let area_drag_id = ui.id().with("preview_area_drag");
    let mut area_drag: Option<AreaDrag> = ui.data(|data| data.get_temp(area_drag_id));
    let mut new_position: Option<(TrackId, SubtitlePosition)> = None;
    // A subtitle layer snaps onto everything except its own lines.
    let area_snap = |track_id: TrackId| {
        let own: Vec<ClipId> = layers
            .iter()
            .filter(|layer| layer.track_id == track_id)
            .map(|layer| layer.clip_id)
            .collect();
        Snap {
            x: Guides::excluding(&guides.x, &own),
            y: Guides::excluding(&guides.y, &own),
        }
    };

    // The selected element takes the mouse first, even where another one
    // lies over it, so dragging always moves what is selected instead of
    // grabbing whatever is on top. A plain click (no drag) still selects
    // the element on top. In egui the area registered last wins, so the
    // selected elements come last.
    let (selected, others): (Vec<&LayerView>, Vec<&LayerView>) = layers
        .iter()
        .partition(|layer| session.is_clip_selected(layer.track_id, layer.clip_id));
    for layer in others.into_iter().chain(selected) {
        let placement = shown_placement(layer, drag, group);
        let rotation = shown_rotation(layer, drag);
        let layer_rect = to_screen(placement, composition_rect);
        let is_selected = session.is_clip_selected(layer.track_id, layer.clip_id);
        if layer.shows_below && !is_selected {
            continue;
        }

        // A turned element takes the mouse only inside its own outline, so
        // the corners of the box around it do not hide the layers below.
        let corners = turned_corners(layer_rect, rotation);
        let reach = Rect::from_points(&corners);
        let id = ui.id().with(("preview_layer", layer.clip_id));
        let is_over_it = ui
            .ctx()
            .pointer_latest_pos()
            .is_some_and(|pointer| is_inside_turned(layer_rect, rotation, pointer));
        let is_held =
            ui.ctx().is_being_dragged(id) || drag.is_some_and(|drag| drag.clip == layer.clip_id);
        let sense = if is_over_it || is_held {
            egui::Sense::click_and_drag()
        } else {
            egui::Sense::hover()
        };
        let response = ui.interact(reach.intersect(panel_rect), id, sense);
        let this = (layer.track_id, layer.clip_id);
        // Picks an element: alone, or (with Shift) in addition to
        // the ones already selected, or out of them if it already was.
        let pick = |session: &mut EditorSession, element: (TrackId, ClipId)| {
            if adds_to_selection && !is_subtitle_layer(session, element.0) {
                session.toggle_clip_selection(element.1);
            } else {
                session.select(selection_for(session, element));
            }
        };
        // Shift + drag draws a selection rectangle even over elements.
        let starts_band = response.drag_started() && ui.input(|input| input.modifiers.shift);
        if starts_band {
            band_start = ui.input(|input| input.pointer.press_origin());
        }
        if response.clicked() {
            // A click picks the element on top, which may be another one
            // than the selected element that took the mouse.
            let pointer = response.interact_pointer_pos().unwrap_or_default();
            let clicked = if is_selected {
                topmost_under(pointer, session).unwrap_or(this)
            } else {
                this
            };
            pick(session, clicked);
        } else if response.drag_started() && !is_selected && !starts_band {
            // Dragging a selected element keeps the selection, so all of
            // the selected elements move together.
            pick(session, this);
        }
        if band_start.is_some() {
            continue;
        }
        let response = response.on_hover_cursor(CursorIcon::Move);
        if let Some(style) = session.project().subtitle_style(layer.track_id) {
            let position = style.position;
            let snap = area_snap(layer.track_id);
            if let Some(moved) = follow_area_drag(
                &response,
                layer.track_id,
                Grab::MOVE,
                placement,
                position,
                (composition_rect, &snap),
                &mut area_drag,
            ) {
                new_position = Some((layer.track_id, moved));
            }
            continue;
        }
        follow_drag(
            &response,
            layer.clip_id,
            Grab::MOVE,
            (placement, rotation),
            (composition_rect, &guides),
            &mut drag,
            &mut finished_drag,
        );
    }

    // Frame and handles of the selected layer, above every picture. Lines
    // of subtitles have none: their layer's area has them (below).
    for layer in layers {
        if !session.is_clip_selected(layer.track_id, layer.clip_id)
            || is_subtitle_layer(session, layer.track_id)
        {
            continue;
        }
        let placement = shown_placement(layer, drag, group);
        let rotation = shown_rotation(layer, drag);
        let layer_rect = to_screen(placement, composition_rect);
        let center = layer_rect.center();
        let corners = turned_corners(layer_rect, rotation);
        painter.add(egui::Shape::closed_line(
            corners.to_vec(),
            Stroke::new(1.0, theme::ACCENT),
        ));
        // With several selected, each shows its outline; resizing and
        // turning are for one element at a time.
        if selected_elements.len() > 1 {
            continue;
        }

        // The rotation handle: a round knob above the top edge.
        let top_middle = center + turned(egui::vec2(0.0, -layer_rect.height() / 2.0), rotation);
        let knob = center
            + turned(
                egui::vec2(0.0, -layer_rect.height() / 2.0 - ROTATION_HANDLE_DISTANCE),
                rotation,
            );
        painter.line_segment([top_middle, knob], Stroke::new(1.0, theme::ACCENT));
        painter.circle(
            knob,
            HANDLE_SIZE / 2.0 + 1.0,
            Color32::WHITE,
            Stroke::new(1.0, theme::ACCENT),
        );
        let response = ui
            .interact(
                Rect::from_center_size(knob, egui::Vec2::splat(HANDLE_SIZE + 8.0)),
                ui.id().with(("preview_rotate", layer.clip_id)),
                egui::Sense::drag(),
            )
            .on_hover_cursor(CursorIcon::Crosshair)
            .on_hover_text("Rotate (Shift: 15\u{b0} steps)");
        follow_drag(
            &response,
            layer.clip_id,
            Grab::ROTATE,
            (placement, rotation),
            (composition_rect, &guides),
            &mut drag,
            &mut finished_drag,
        );

        for (grab, unturned_position, cursor) in handles(layer_rect) {
            let position = center + turned(unturned_position - center, rotation);
            let handle_rect = Rect::from_center_size(position, egui::Vec2::splat(HANDLE_SIZE));
            let response = ui
                .interact(
                    // A little larger than drawn, so it is easier to hit.
                    handle_rect.expand(3.0),
                    ui.id().with(("preview_handle", layer.clip_id, grab)),
                    egui::Sense::drag(),
                )
                .on_hover_cursor(cursor);
            painter.rect_filled(handle_rect, 0.0, Color32::WHITE);
            painter.rect_stroke(
                handle_rect,
                0.0,
                Stroke::new(1.0, theme::ACCENT),
                StrokeKind::Inside,
            );
            follow_drag(
                &response,
                layer.clip_id,
                grab,
                (placement, rotation),
                (composition_rect, &guides),
                &mut drag,
                &mut finished_drag,
            );
        }
    }

    // The area of the selected subtitle layer: where its lines may appear
    // (random places), or the point they are centered on (fixed place).
    // Drag inside or on a line to move it, drag a handle to resize it.
    let selected_subtitles = session
        .selected_track()
        .filter(|track| track.kind == TrackKind::Subtitles)
        .and_then(|track| Some((track.id, track.subtitle_style.as_ref()?.position)));
    if let Some((track_id, position)) = selected_subtitles {
        let snap = area_snap(track_id);
        let mut follow = |response: &egui::Response, grab: Grab, shape: Placement| {
            if let Some(moved) = follow_area_drag(
                response,
                track_id,
                grab,
                shape,
                position,
                (composition_rect, &snap),
                &mut area_drag,
            ) {
                new_position = Some((track_id, moved));
            }
        };
        match position {
            SubtitlePosition::Random { area } => {
                let area_rect = to_screen(area, composition_rect);
                painter.rect_filled(area_rect, 0.0, theme::ACCENT.gamma_multiply(0.08));
                let corners = [
                    area_rect.left_top(),
                    area_rect.right_top(),
                    area_rect.right_bottom(),
                    area_rect.left_bottom(),
                    area_rect.left_top(),
                ];
                painter.extend(egui::Shape::dashed_line(
                    &corners,
                    Stroke::new(1.5, theme::ACCENT),
                    6.0,
                    4.0,
                ));
                painter.text(
                    area_rect.left_top() + egui::vec2(6.0, 4.0),
                    Align2::LEFT_TOP,
                    "Lyrics area",
                    FontId::proportional(11.0),
                    theme::ACCENT,
                );
                // Inside: move. Clicking (not dragging) still selects what
                // is under the mouse, so the area never hides other
                // elements.
                let inside = ui
                    .interact(
                        area_rect.intersect(panel_rect),
                        ui.id().with(("preview_area", track_id)),
                        egui::Sense::click_and_drag(),
                    )
                    .on_hover_cursor(CursorIcon::Move);
                if inside.clicked() {
                    let pointer = inside.interact_pointer_pos().unwrap_or_default();
                    match topmost_under(pointer, session) {
                        Some(element) => session.select(selection_for(session, element)),
                        None => session.select(Selection::Track(track_id)),
                    }
                }
                follow(&inside, Grab::MOVE, area);
                for (grab, position, cursor) in handles(area_rect) {
                    let handle_rect =
                        Rect::from_center_size(position, egui::Vec2::splat(HANDLE_SIZE));
                    let response = ui
                        .interact(
                            handle_rect.expand(3.0),
                            ui.id().with(("preview_area_handle", track_id, grab)),
                            egui::Sense::drag(),
                        )
                        .on_hover_cursor(cursor);
                    painter.rect_filled(handle_rect, 0.0, Color32::WHITE);
                    painter.rect_stroke(
                        handle_rect,
                        0.0,
                        Stroke::new(1.0, theme::ACCENT),
                        StrokeKind::Inside,
                    );
                    follow(&response, grab, area);
                }
            }
            SubtitlePosition::Fixed { x, y } => {
                let point = composition_rect.left_top()
                    + egui::vec2(
                        x as f32 * composition_rect.width(),
                        y as f32 * composition_rect.height(),
                    );
                let stroke = Stroke::new(1.5, theme::ACCENT);
                painter.circle(point, 6.0, Color32::from_black_alpha(120), stroke);
                painter.hline((point.x - 10.0)..=(point.x + 10.0), point.y, stroke);
                painter.vline(point.x, (point.y - 10.0)..=(point.y + 10.0), stroke);
                let response = ui
                    .interact(
                        Rect::from_center_size(point, egui::Vec2::splat(18.0)),
                        ui.id().with(("preview_point", track_id)),
                        egui::Sense::drag(),
                    )
                    .on_hover_cursor(CursorIcon::Move)
                    .on_hover_text("Where the lines are centered: drag to move");
                // A small square around the point, so its middle snaps.
                let half = 0.01;
                let shape = Placement {
                    x: x - half,
                    y: y - half,
                    width: 2.0 * half,
                    height: 2.0 * half,
                };
                follow(&response, Grab::MOVE, shape);
            }
        }
    }
    if let Some((track_id, position)) = new_position
        && let Some(style) = session.project().subtitle_style(track_id).cloned()
    {
        // Written while dragging, so the lines follow the mouse; the undo
        // history makes the whole drag one step.
        session.edit_project(|project| {
            project.set_subtitle_style(track_id, SubtitleStyle { position, ..style })
        });
    }
    ui.data_mut(|data| match area_drag {
        Some(active) => {
            data.insert_temp(area_drag_id, active);
        }
        None => data.remove::<AreaDrag>(area_drag_id),
    });

    // The guides the dragged element is snapped onto, across the frame.
    let snapped = drag
        .map(|drag| drag.snapped)
        .or(area_drag.map(|drag| drag.snapped));
    if let Some((snapped_x, snapped_y)) = snapped {
        let line = Stroke::new(1.0, GUIDE_COLOR);
        let frame = composition_rect;
        if let Some(x) = snapped_x {
            let x = frame.left() + x as f32 * frame.width();
            painter.vline(x, frame.y_range(), line);
        }
        if let Some(y) = snapped_y {
            let y = frame.top() + y as f32 * frame.height();
            painter.hline(frame.x_range(), y, line);
        }
    }

    // The selection rectangle: drawn while the mouse is held, and on
    // release it selects every element lying entirely inside it.
    if let Some(from) = band_start {
        let to = ui.input(|input| input.pointer.latest_pos()).unwrap_or(from);
        let band = Rect::from_two_pos(from, to);
        painter.rect_filled(band, 0.0, theme::ACCENT.gamma_multiply(0.12));
        painter.rect_stroke(
            band,
            0.0,
            Stroke::new(1.0, theme::ACCENT),
            StrokeKind::Inside,
        );
        if !ui.input(|input| input.pointer.primary_down()) {
            band_start = None;
            let inside = elements_inside(layers, composition_rect, (from, to))
                .into_iter()
                .filter(|clip| {
                    // Lines of subtitles belong to their layer.
                    layers
                        .iter()
                        .find(|layer| layer.clip_id == *clip)
                        .is_some_and(|layer| !is_subtitle_layer(session, layer.track_id))
                });
            let mut chosen = if adds_to_selection {
                session.selected_clip_ids()
            } else {
                Vec::new()
            };
            for clip in inside {
                if !chosen.contains(&clip) {
                    chosen.push(clip);
                }
            }
            session.select_clips(&chosen);
        }
    }
    ui.data_mut(|data| match band_start {
        Some(from) => {
            data.insert_temp(band_id, from);
        }
        None => data.remove::<egui::Pos2>(band_id),
    });

    // While dragging, the picture itself follows the mouse too, with the
    // other selected elements when the drag moves them.
    let moving_along = GroupMove::of(drag, &selected_elements);
    let mut dragged_placements = Vec::new();
    if let Some(active) = drag {
        dragged_placements.push((active.clip, active.current, active.rotation));
        for layer in layers {
            if moving_along.clips.contains(&layer.clip_id) {
                dragged_placements.push((
                    layer.clip_id,
                    moving_along.moved(layer.placement),
                    layer.rotation_degrees,
                ));
            }
        }
    }
    visual_preview::set_dragged_placements(ui.ctx(), dragged_placements);
    ui.data_mut(|data| match drag {
        Some(drag) => {
            data.insert_temp(drag_id, drag);
        }
        None => data.remove::<Drag>(drag_id),
    });
    if let Some(finished) = finished_drag {
        // Changes the clip at the playhead: its resting place, or the
        // keyframe there (a new one if the clip has keyframes elsewhere).
        // The other selected elements moved with it change the same way.
        let playhead_milliseconds = (session.playhead_seconds() * 1000.0).round() as u64;
        let (shift_x, shift_y) = drag_shift(&finished);
        let along = GroupMove::of(Some(finished), &selected_elements).clips;
        session.edit_project(|project| {
            let mut has_changed =
                project.edit_clip_transform_at(finished.clip, playhead_milliseconds, |transform| {
                    transform.placement = finished.current;
                    transform.rotation_degrees = finished.rotation;
                });
            for clip in &along {
                has_changed |=
                    project.edit_clip_transform_at(*clip, playhead_milliseconds, |transform| {
                        transform.placement.x += shift_x;
                        transform.placement.y += shift_y;
                    });
            }
            has_changed
        });
    }

    painter.text(
        composition_rect.left_bottom() + egui::vec2(6.0, -6.0),
        Align2::LEFT_BOTTOM,
        format!(
            "{} x {}  |  {:.2} fps",
            settings.width,
            settings.height,
            settings.frame_rate.as_f64()
        ),
        FontId::monospace(11.0),
        theme::TEXT_MUTED,
    );
}

/// Which edges of a placement follow the mouse during a drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Grab {
    left: bool,
    top: bool,
    right: bool,
    bottom: bool,
}

impl Grab {
    /// All four edges move together: the clip keeps its size.
    const MOVE: Self = Self {
        left: true,
        top: true,
        right: true,
        bottom: true,
    };
    /// No edge moves: the clip turns around its middle.
    const ROTATE: Self = Self {
        left: false,
        top: false,
        right: false,
        bottom: false,
    };
}

/// A move or resize that is in progress.
#[derive(Debug, Clone, Copy)]
struct Drag {
    clip: ClipId,
    grab: Grab,
    /// Placement when the drag began.
    start: Placement,
    /// How far the mouse has moved since then, in screen points.
    offset: egui::Vec2,
    /// Placement the drag has produced so far.
    current: Placement,
    /// Rotation, in degrees, when the drag began and so far.
    start_rotation: f64,
    rotation: f64,
    /// Where the mouse took hold, in screen points.
    start_pointer: egui::Pos2,
    /// The vertical and horizontal guide (fractions of the frame) the
    /// element is snapped onto right now, shown as lines.
    snapped: (Option<f64>, Option<f64>),
}

/// Lines a dragged element snaps onto, as fractions of the frame: the
/// frame's edges and middle, and the edges and middles of the elements in
/// the picture (each with its clip, so an element never snaps onto itself).
#[derive(Debug, Clone, Default)]
struct Guides {
    x: Vec<(f64, Option<ClipId>)>,
    y: Vec<(f64, Option<ClipId>)>,
}

impl Guides {
    /// Only the frame's edges and middle.
    fn frame() -> Self {
        let frame = vec![(0.0, None), (0.5, None), (1.0, None)];
        Self {
            x: frame.clone(),
            y: frame,
        }
    }

    /// The frame and every upright element of the picture (a turned one has
    /// no straight edges to line up with).
    fn of_picture(layers: &[LayerView]) -> Self {
        let mut guides = Self::frame();
        for layer in layers {
            let is_upright = layer.rotation_degrees.rem_euclid(360.0).abs() < 1e-9;
            if !is_upright || layer.shows_below {
                continue;
            }
            let p = layer.placement;
            let clip = Some(layer.clip_id);
            guides
                .x
                .extend([(p.x, clip), (p.x + p.width / 2.0, clip), (p.right(), clip)]);
            guides.y.extend([
                (p.y, clip),
                (p.y + p.height / 2.0, clip),
                (p.bottom(), clip),
            ]);
        }
        guides
    }

    /// The same guides without those of the clips `moving` (selected
    /// elements that move together never snap onto each other).
    fn without(mut self, moving: &[ClipId]) -> Self {
        let keep =
            |(_, owner): &(f64, Option<ClipId>)| owner.is_none_or(|owner| !moving.contains(&owner));
        self.x.retain(keep);
        self.y.retain(keep);
        self
    }

    /// The guide positions along one axis, leaving out those of the clips
    /// `own` (the lines of a subtitle layer whose area is dragged).
    fn excluding(list: &[(f64, Option<ClipId>)], own: &[ClipId]) -> Vec<f64> {
        list.iter()
            .filter(|(_, owner)| owner.is_none_or(|owner| !own.contains(&owner)))
            .map(|(value, _)| *value)
            .collect()
    }

    /// The guide positions along one axis that `clip` may snap onto.
    fn along(list: &[(f64, Option<ClipId>)], clip: ClipId) -> Vec<f64> {
        list.iter()
            .filter(|(_, owner)| *owner != Some(clip))
            .map(|(value, _)| *value)
            .collect()
    }
}

/// A move or resize of a subtitle layer's area (or point) in progress.
/// Unlike a clip, the layer is changed while dragging, so its lines follow
/// the mouse.
#[derive(Debug, Clone, Copy)]
struct AreaDrag {
    track: TrackId,
    grab: Grab,
    /// The rectangle that follows the mouse, when the drag began: the area
    /// itself, or the line that was grabbed (moving it moves the area).
    shape: Placement,
    /// The layer's position when the drag began.
    start: SubtitlePosition,
    /// How far the mouse has moved since then, in screen points.
    offset: egui::Vec2,
    /// The guides snapped onto right now (across, down).
    snapped: (Option<f64>, Option<f64>),
}

/// Starts, continues or ends a drag of a subtitle layer's area according
/// to what happened to `response` this frame. Returns the layer's new
/// position while it is dragged.
fn follow_area_drag(
    response: &egui::Response,
    track: TrackId,
    grab: Grab,
    shape: Placement,
    position: SubtitlePosition,
    (composition_rect, snap): (Rect, &Snap),
    area_drag: &mut Option<AreaDrag>,
) -> Option<SubtitlePosition> {
    if response.drag_started() {
        *area_drag = Some(AreaDrag {
            track,
            grab,
            shape,
            start: position,
            offset: egui::Vec2::ZERO,
            snapped: (None, None),
        });
    }
    let is_this_drag = |drag: &AreaDrag| drag.track == track && drag.grab == grab;
    let mut moved_to = None;
    if response.dragged()
        && let Some(active) = area_drag.as_mut().filter(|drag| is_this_drag(drag))
    {
        active.offset += response.drag_delta();
        let (moved, snapped) = dragged_placement(
            active.shape,
            grab,
            active.offset,
            composition_rect,
            Some(snap),
        );
        active.snapped = snapped;
        moved_to = Some(moved_position(active.start, grab, active.shape, moved));
    }
    if response.drag_stopped() && area_drag.as_ref().is_some_and(is_this_drag) {
        *area_drag = None;
    }
    moved_to
}

/// A subtitle layer's position after the rectangle `shape` (its area, or
/// one of its lines) was dragged to `moved` with `grab`. It stays inside
/// the frame.
fn moved_position(
    start: SubtitlePosition,
    grab: Grab,
    shape: Placement,
    moved: Placement,
) -> SubtitlePosition {
    let (shift_x, shift_y) = (moved.x - shape.x, moved.y - shape.y);
    match start {
        SubtitlePosition::Fixed { x, y } => SubtitlePosition::Fixed {
            x: (x + shift_x).clamp(0.0, 1.0),
            y: (y + shift_y).clamp(0.0, 1.0),
        },
        SubtitlePosition::Random { area } if grab == Grab::MOVE => {
            // Kept inside the frame without changing its size.
            let x = (area.x + shift_x).clamp(0.0, (1.0 - area.width).max(0.0));
            let y = (area.y + shift_y).clamp(0.0, (1.0 - area.height).max(0.0));
            SubtitlePosition::Random {
                area: Placement { x, y, ..area },
            }
        }
        SubtitlePosition::Random { .. } => {
            // Resized: within the frame.
            let left = moved.x.clamp(0.0, 1.0);
            let top = moved.y.clamp(0.0, 1.0);
            let right = moved.right().clamp(left, 1.0);
            let bottom = moved.bottom().clamp(top, 1.0);
            SubtitlePosition::Random {
                area: Placement {
                    x: left,
                    y: top,
                    width: (right - left).max(Placement::MIN_SIZE),
                    height: (bottom - top).max(Placement::MIN_SIZE),
                },
            }
        }
    }
}

/// The other selected elements that move along with the one being
/// dragged, and how far they have moved (fractions of the frame).
#[derive(Debug, Clone, Default)]
struct GroupMove {
    clips: Vec<ClipId>,
    shift: (f64, f64),
}

impl GroupMove {
    /// While `drag` moves one of the `selected` elements, the others move
    /// with it by the same amount. Resizing and turning stay with one.
    fn of(drag: Option<Drag>, selected: &[ClipId]) -> Self {
        match drag {
            Some(drag) if drag.grab == Grab::MOVE && selected.contains(&drag.clip) => Self {
                clips: selected
                    .iter()
                    .copied()
                    .filter(|clip| *clip != drag.clip)
                    .collect(),
                shift: drag_shift(&drag),
            },
            _ => Self::default(),
        }
    }

    fn moved(&self, placement: Placement) -> Placement {
        Placement {
            x: placement.x + self.shift.0,
            y: placement.y + self.shift.1,
            ..placement
        }
    }
}

/// How far a drag has moved its element so far (fractions of the frame).
fn drag_shift(drag: &Drag) -> (f64, f64) {
    (drag.current.x - drag.start.x, drag.current.y - drag.start.y)
}

/// The placement to draw a layer with: where the drag has taken it, if it
/// is being dragged (or moves along with the dragged one), otherwise the
/// one stored in the project.
fn shown_placement(layer: &LayerView, drag: Option<Drag>, group: &GroupMove) -> Placement {
    match drag {
        Some(drag) if drag.clip == layer.clip_id => drag.current,
        _ if group.clips.contains(&layer.clip_id) => group.moved(layer.placement),
        _ => layer.placement,
    }
}

/// Which elements a selection rectangle from `from` to `to` (screen
/// points) takes in: those lying entirely inside it.
fn elements_inside(
    layers: &[LayerView],
    composition_rect: Rect,
    (from, to): (egui::Pos2, egui::Pos2),
) -> Vec<ClipId> {
    let band = Rect::from_two_pos(from, to);
    layers
        .iter()
        .filter(|layer| !layer.shows_below)
        .filter(|layer| {
            let rect = to_screen(layer.placement, composition_rect);
            let corners = turned_corners(rect, layer.rotation_degrees);
            corners.iter().all(|corner| band.contains(*corner))
        })
        .map(|layer| layer.clip_id)
        .collect()
}

/// The rotation to draw a layer with: the one being dragged, if it is
/// being dragged, otherwise the one stored in the project.
fn shown_rotation(layer: &LayerView, drag: Option<Drag>) -> f64 {
    match drag {
        Some(drag) if drag.clip == layer.clip_id => drag.rotation,
        _ => layer.rotation_degrees,
    }
}

/// Starts, continues or ends a drag according to what happened to
/// `response` this frame.
fn follow_drag(
    response: &egui::Response,
    clip: ClipId,
    grab: Grab,
    (placement, rotation): (Placement, f64),
    (composition_rect, guides): (Rect, &Guides),
    drag: &mut Option<Drag>,
    finished_drag: &mut Option<Drag>,
) {
    if response.drag_started() {
        *drag = Some(Drag {
            clip,
            grab,
            start: placement,
            offset: egui::Vec2::ZERO,
            current: placement,
            start_rotation: rotation,
            rotation,
            start_pointer: response.interact_pointer_pos().unwrap_or_default(),
            snapped: (None, None),
        });
    }
    let is_this_drag = |drag: &Drag| drag.clip == clip && drag.grab == grab;

    if response.dragged()
        && let Some(active) = drag.as_mut().filter(|active| is_this_drag(active))
    {
        active.offset += response.drag_delta();
        if grab == Grab::ROTATE {
            let center = to_screen(active.start, composition_rect).center();
            let pointer = active.start_pointer + active.offset;
            let snap_degrees = response
                .ctx
                .input(|input| input.modifiers.shift)
                .then_some(15.0);
            active.rotation = dragged_rotation(
                active.start_rotation,
                active.start_pointer - center,
                pointer - center,
                snap_degrees,
            );
        } else {
            let guides = Snap {
                x: Guides::along(&guides.x, clip),
                y: Guides::along(&guides.y, clip),
            };
            (active.current, active.snapped) = turned_dragged_placement(
                active.start,
                active.start_rotation,
                grab,
                active.offset,
                composition_rect,
                &guides,
            );
        }
    }
    if response.drag_stopped() && drag.as_ref().is_some_and(is_this_drag) {
        *finished_drag = drag.take();
    }
}

/// The rotation after the mouse has gone round the middle from `from` to
/// `to` (both relative to the middle, in screen points), in degrees
/// clockwise, optionally in steps of `snap_degrees`.
fn dragged_rotation(
    start_rotation: f64,
    from: egui::Vec2,
    to: egui::Vec2,
    snap_degrees: Option<f64>,
) -> f64 {
    let angle = |vector: egui::Vec2| f64::from(vector.y).atan2(f64::from(vector.x));
    // Angles in screen space grow clockwise, because y grows downwards.
    let mut turn = (angle(to) - angle(from)).to_degrees();
    // The shortest way round, so crossing the left side does not jump.
    turn = (turn + 180.0).rem_euclid(360.0) - 180.0;
    let rotation = start_rotation + turn;
    match snap_degrees {
        Some(step) if step > 0.0 => (rotation / step).round() * step,
        _ => rotation,
    }
}

/// Like [`dragged_placement`], for an element turned by `rotation`
/// degrees: the mouse movement is measured along the element's own edges,
/// and the opposite edge stays where it is on the screen.
fn turned_dragged_placement(
    start: Placement,
    rotation: f64,
    grab: Grab,
    offset: egui::Vec2,
    composition_rect: Rect,
    guides: &Snap,
) -> (Placement, (Option<f64>, Option<f64>)) {
    let is_turned = rotation.rem_euclid(360.0).abs() > 1e-9;
    if grab == Grab::MOVE || !is_turned {
        // Guides along the frame's edges only make sense for an upright
        // element.
        let guides = (!is_turned).then_some(guides);
        return dragged_placement(start, grab, offset, composition_rect, guides);
    }
    let local_offset = turned(offset, -rotation);
    let (upright, _) = dragged_placement(start, grab, local_offset, composition_rect, None);

    // The upright result has its middle moved along the screen's axes;
    // turn that move to follow the element's own axes.
    let start_center = to_screen(start, composition_rect).center();
    let moved = to_screen(upright, composition_rect).center() - start_center;
    let center = start_center + turned(moved, rotation);
    let placement = Placement {
        x: f64::from((center.x - composition_rect.left()) / composition_rect.width())
            - upright.width / 2.0,
        y: f64::from((center.y - composition_rect.top()) / composition_rect.height())
            - upright.height / 2.0,
        ..upright
    }
    .sanitized();
    (placement, (None, None))
}

/// `vector` turned by `degrees` clockwise on the screen.
fn turned(vector: egui::Vec2, degrees: f64) -> egui::Vec2 {
    let (sin, cos) = (degrees.to_radians() as f32).sin_cos();
    egui::vec2(
        vector.x * cos - vector.y * sin,
        vector.x * sin + vector.y * cos,
    )
}

/// The four corners of `rect` turned by `degrees` around its middle.
fn turned_corners(rect: Rect, degrees: f64) -> [egui::Pos2; 4] {
    let center = rect.center();
    [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ]
    .map(|corner| center + turned(corner - center, degrees))
}

/// Whether `point` lies inside `rect` turned by `degrees` around its middle.
fn is_inside_turned(rect: Rect, degrees: f64, point: egui::Pos2) -> bool {
    let center = rect.center();
    let unturned = center + turned(point - center, -degrees);
    rect.contains(unturned)
}

/// The guide positions (fractions of the frame) one dragged element may
/// snap onto, across and down.
#[derive(Debug, Clone, Default)]
struct Snap {
    x: Vec<f64>,
    y: Vec<f64>,
}

/// The placement that results from dragging the grabbed edges of `start`
/// by `offset` screen points, and the guides it snapped onto (across,
/// down). With `guides`, an edge (or, when moving, the middle) jumps onto
/// the nearest guide that comes close.
fn dragged_placement(
    start: Placement,
    grab: Grab,
    offset: egui::Vec2,
    composition_rect: Rect,
    guides: Option<&Snap>,
) -> (Placement, (Option<f64>, Option<f64>)) {
    if composition_rect.width() <= 0.0 || composition_rect.height() <= 0.0 {
        return (start, (None, None));
    }
    // Screen points to fractions of the composition.
    let delta_x = f64::from(offset.x / composition_rect.width());
    let delta_y = f64::from(offset.y / composition_rect.height());
    let snap_distance = if guides.is_some() { SNAP_DISTANCE } else { 0.0 };
    let snap_x = f64::from(snap_distance / composition_rect.width());
    let snap_y = f64::from(snap_distance / composition_rect.height());
    let no_guides = Snap::default();
    let guides = guides.unwrap_or(&no_guides);

    let (left, right, snapped_x) = dragged_axis(
        start.x,
        start.right(),
        (grab.left, grab.right),
        delta_x,
        (&guides.x, snap_x),
    );
    let (top, bottom, snapped_y) = dragged_axis(
        start.y,
        start.bottom(),
        (grab.top, grab.bottom),
        delta_y,
        (&guides.y, snap_y),
    );
    let placement = Placement {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
    .sanitized();
    (placement, (snapped_x, snapped_y))
}

/// Moves the grabbed ends of one axis (`low`..`high`) by `delta`, snapping
/// onto the nearest of `guides` within `snap_distance` and keeping the
/// minimum size. Returns the new ends and the guide snapped onto.
fn dragged_axis(
    low: f64,
    high: f64,
    (moves_low, moves_high): (bool, bool),
    delta: f64,
    (guides, snap_distance): (&[f64], f64),
) -> (f64, f64, Option<f64>) {
    // The nearest guide to any of `points` (each with how far the guide is
    // from where the element's low end then goes).
    let nearest = |points: &[(f64, f64)]| {
        points
            .iter()
            .flat_map(|(point, to_low)| {
                guides
                    .iter()
                    .map(move |guide| ((guide - point).abs(), *guide, guide - to_low))
            })
            .filter(|(distance, _, _)| *distance <= snap_distance)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, guide, new_low)| (guide, new_low))
    };

    match (moves_low, moves_high) {
        // Moving: the size stays; the end or middle nearest a guide wins.
        (true, true) => {
            let size = high - low;
            let moved_low = low + delta;
            let points = [
                (moved_low, 0.0),
                (moved_low + size / 2.0, size / 2.0),
                (moved_low + size, size),
            ];
            match nearest(&points) {
                Some((guide, new_low)) => (new_low, new_low + size, Some(guide)),
                None => (moved_low, moved_low + size, None),
            }
        }
        (true, false) => {
            let moved = low + delta;
            let snapped = nearest(&[(moved, 0.0)]).map(|(guide, _)| guide);
            let edge = snapped.unwrap_or(moved).min(high - Placement::MIN_SIZE);
            (edge, high, snapped.filter(|guide| *guide == edge))
        }
        (false, true) => {
            let moved = high + delta;
            let snapped = nearest(&[(moved, 0.0)]).map(|(guide, _)| guide);
            let edge = snapped.unwrap_or(moved).max(low + Placement::MIN_SIZE);
            (low, edge, snapped.filter(|guide| *guide == edge))
        }
        (false, false) => (low, high, None),
    }
}

/// The eight resize handles of a rectangle: which edges each one moves,
/// where it is, and the mouse cursor that announces it.
fn handles(rect: Rect) -> [(Grab, egui::Pos2, CursorIcon); 8] {
    let grab = |left, top, right, bottom| Grab {
        left,
        top,
        right,
        bottom,
    };
    let center = rect.center();
    [
        (
            grab(true, true, false, false),
            rect.left_top(),
            CursorIcon::ResizeNwSe,
        ),
        (
            grab(false, true, false, false),
            egui::pos2(center.x, rect.top()),
            CursorIcon::ResizeVertical,
        ),
        (
            grab(false, true, true, false),
            rect.right_top(),
            CursorIcon::ResizeNeSw,
        ),
        (
            grab(false, false, true, false),
            egui::pos2(rect.right(), center.y),
            CursorIcon::ResizeHorizontal,
        ),
        (
            grab(false, false, true, true),
            rect.right_bottom(),
            CursorIcon::ResizeNwSe,
        ),
        (
            grab(false, false, false, true),
            egui::pos2(center.x, rect.bottom()),
            CursorIcon::ResizeVertical,
        ),
        (
            grab(true, false, false, true),
            rect.left_bottom(),
            CursorIcon::ResizeNeSw,
        ),
        (
            grab(true, false, false, false),
            egui::pos2(rect.left(), center.y),
            CursorIcon::ResizeHorizontal,
        ),
    ]
}

/// The screen rectangle of a placement inside the composition frame.
fn to_screen(placement: Placement, composition_rect: Rect) -> Rect {
    let size = composition_rect.size();
    Rect::from_min_size(
        composition_rect.min + egui::vec2(placement.x as f32 * size.x, placement.y as f32 * size.y),
        egui::vec2(
            placement.width as f32 * size.x,
            placement.height as f32 * size.y,
        ),
    )
}

/// For a clip that is not in the picture yet, draws a dark box with the
/// reason, so the clip's place in the frame is still visible.
fn draw_layer_state(painter: &egui::Painter, layer_rect: Rect, state: &LayerState) {
    let message = match state {
        LayerState::Shown => return,
        LayerState::Rendering { finished, total } if *total > 0 => format!(
            "Pre-rendering: {}%\n({finished} of {total} frames)",
            u64::from(*finished) * 100 / u64::from(*total)
        ),
        LayerState::Rendering { .. } => "Queued for pre-render...".to_owned(),
        LayerState::WaitingForAudio => "Loading audio...".to_owned(),
        LayerState::LoadingImage => "Loading image...".to_owned(),
        LayerState::Failed(reason) => reason.clone(),
    };
    painter.rect_filled(layer_rect, 0.0, Color32::from_gray(10));
    painter.rect_stroke(
        layer_rect,
        0.0,
        Stroke::new(1.0, theme::SEPARATOR),
        StrokeKind::Inside,
    );
    draw_message(painter, layer_rect, message);
}

/// Text centered in a rectangle.
fn draw_message(painter: &egui::Painter, rect: Rect, message: String) {
    let galley = painter.layout(
        message,
        FontId::proportional(13.0),
        theme::TEXT_MUTED,
        (rect.width() - 24.0).max(40.0),
    );
    painter.galley(
        rect.center() - galley.size() / 2.0,
        galley,
        theme::TEXT_MUTED,
    );
}

/// Largest rectangle with the aspect ratio of `content_size` that fits
/// centered inside `bounds`.
fn fit_inside(content_size: egui::Vec2, bounds: Rect) -> Rect {
    if content_size.x <= 0.0 || content_size.y <= 0.0 || !bounds.is_positive() {
        return Rect::from_center_size(bounds.center(), egui::Vec2::ZERO);
    }

    let scale = (bounds.width() / content_size.x).min(bounds.height() / content_size.y);
    Rect::from_center_size(bounds.center(), content_size * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1000 x 500 point frame, so one point is 0.001 / 0.002 of it.
    fn frame() -> Rect {
        Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(1000.0, 500.0))
    }

    fn frame_guides() -> Snap {
        Snap {
            x: vec![0.0, 0.5, 1.0],
            y: vec![0.0, 0.5, 1.0],
        }
    }

    #[test]
    fn a_moved_element_snaps_its_middle_and_onto_other_elements() {
        let start = placement(0.1, 0.1, 0.2, 0.2);
        // Its middle (0.2) moved by 297 points lands 3 points from the
        // frame's middle: it snaps there and the vertical guide shows.
        let (moved, snapped) = dragged_placement(
            start,
            Grab::MOVE,
            egui::vec2(297.0, 0.0),
            frame(),
            Some(&frame_guides()),
        );
        assert_close(moved, placement(0.4, 0.1, 0.2, 0.2));
        assert_eq!(snapped, (Some(0.5), None));

        // Another element's right edge at 0.73 is a guide too.
        let guides = Snap {
            x: vec![0.73],
            y: Vec::new(),
        };
        let (moved, snapped) = dragged_placement(
            start,
            Grab::MOVE,
            egui::vec2(632.0, 0.0),
            frame(),
            Some(&guides),
        );
        assert_close(moved, placement(0.73, 0.1, 0.2, 0.2));
        assert_eq!(snapped.0, Some(0.73));

        // Without guides (Alt held) nothing snaps.
        let (free, snapped) =
            dragged_placement(start, Grab::MOVE, egui::vec2(297.0, 0.0), frame(), None);
        assert_close(free, placement(0.397, 0.1, 0.2, 0.2));
        assert_eq!(snapped, (None, None));
    }

    fn placement(x: f64, y: f64, width: f64, height: f64) -> Placement {
        Placement {
            x,
            y,
            width,
            height,
        }
    }

    fn assert_close(actual: Placement, expected: Placement) {
        let values = |p: Placement| [p.x, p.y, p.width, p.height];
        for (a, e) in values(actual).into_iter().zip(values(expected)) {
            assert!((a - e).abs() < 1e-6, "{actual:?} != {expected:?}");
        }
    }

    #[test]
    fn a_turned_element_is_hit_only_inside_its_outline() {
        let rect = Rect::from_center_size(egui::pos2(100.0, 100.0), egui::vec2(100.0, 20.0));
        // Turned a quarter: now tall and thin.
        assert!(is_inside_turned(rect, 90.0, egui::pos2(100.0, 140.0)));
        assert!(!is_inside_turned(rect, 90.0, egui::pos2(140.0, 100.0)));
        // At 45 degrees the corner of the box around it is empty.
        assert!(!is_inside_turned(rect, 45.0, egui::pos2(60.0, 60.0)));
        assert!(is_inside_turned(rect, 45.0, egui::pos2(120.0, 120.0)));
    }

    #[test]
    fn turning_follows_the_mouse_around_the_middle() {
        let right = egui::vec2(10.0, 0.0);
        // Screen y grows downwards, so "down" is a quarter turn clockwise.
        assert!((dragged_rotation(0.0, right, egui::vec2(0.0, 10.0), None) - 90.0).abs() < 1e-4);
        assert!((dragged_rotation(30.0, right, egui::vec2(0.0, -10.0), None) + 60.0).abs() < 1e-4);
        // Across the left side it takes the short way, not almost a full
        // turn the other way.
        let crossing = dragged_rotation(0.0, egui::vec2(-10.0, 1.0), egui::vec2(-10.0, -1.0), None);
        assert!(crossing > 0.0 && crossing < 15.0, "{crossing}");
        // With steps of 15 degrees.
        let snapped = dragged_rotation(0.0, right, egui::vec2(10.0, 7.5), Some(15.0));
        assert_eq!(snapped, 30.0);
    }

    #[test]
    fn a_turned_element_grows_along_its_own_edges() {
        let start = placement(0.4, 0.4, 0.2, 0.2); // 200 x 100 points
        let right_edge = Grab {
            left: false,
            top: false,
            right: true,
            bottom: false,
        };

        // Turned a quarter clockwise, its own right edge faces down: moving
        // the mouse down by 50 points makes it 50 points (0.05) wider, and
        // its middle moves down by half of that.
        let grown = turned_dragged_placement(
            start,
            90.0,
            right_edge,
            egui::vec2(0.0, 50.0),
            frame(),
            &frame_guides(),
        )
        .0;
        assert_close(grown, placement(0.375, 0.45, 0.25, 0.2));

        // Moving the mouse sideways does not resize it.
        let sideways = turned_dragged_placement(
            start,
            90.0,
            right_edge,
            egui::vec2(50.0, 0.0),
            frame(),
            &frame_guides(),
        )
        .0;
        assert_close(sideways, start);

        // Upright, it is the ordinary resize.
        let upright = turned_dragged_placement(
            start,
            0.0,
            right_edge,
            egui::vec2(50.0, 0.0),
            frame(),
            &frame_guides(),
        )
        .0;
        assert_close(upright, placement(0.4, 0.4, 0.25, 0.2));
    }

    #[test]
    fn moving_keeps_the_size() {
        let start = placement(0.2, 0.2, 0.3, 0.4);

        let moved = dragged_placement(
            start,
            Grab::MOVE,
            egui::vec2(100.0, -50.0),
            frame(),
            Some(&frame_guides()),
        )
        .0;

        assert_close(moved, placement(0.3, 0.1, 0.3, 0.4));
    }

    #[test]
    fn a_corner_handle_moves_only_its_two_edges() {
        let start = placement(0.2, 0.2, 0.3, 0.4);
        let bottom_right = Grab {
            left: false,
            top: false,
            right: true,
            bottom: true,
        };

        let resized = dragged_placement(
            start,
            bottom_right,
            egui::vec2(100.0, 50.0),
            frame(),
            Some(&frame_guides()),
        )
        .0;

        assert_close(resized, placement(0.2, 0.2, 0.4, 0.5));
    }

    #[test]
    fn edges_snap_to_the_frame_and_its_middle() {
        let start = placement(0.2, 0.2, 0.3, 0.4);
        let right_edge = Grab {
            left: false,
            top: false,
            right: true,
            bottom: false,
        };
        // The right edge is at 0.5 already; 4 points away is within reach
        // of the middle guide, so it stays there.
        let nudged = dragged_placement(
            start,
            right_edge,
            egui::vec2(4.0, 0.0),
            frame(),
            Some(&frame_guides()),
        )
        .0;
        assert_close(nudged, start);

        // Moving left by 197 points puts the left edge 3 points from the
        // frame edge: it snaps onto it, and the size is kept.
        let moved = dragged_placement(
            start,
            Grab::MOVE,
            egui::vec2(-197.0, 0.0),
            frame(),
            Some(&frame_guides()),
        )
        .0;
        assert_close(moved, placement(0.0, 0.2, 0.3, 0.4));
    }

    #[test]
    fn an_edge_cannot_be_dragged_past_the_opposite_one() {
        let start = placement(0.2, 0.2, 0.3, 0.4);
        let left_edge = Grab {
            left: true,
            top: false,
            right: false,
            bottom: false,
        };

        let squeezed = dragged_placement(
            start,
            left_edge,
            egui::vec2(900.0, 0.0),
            frame(),
            Some(&frame_guides()),
        )
        .0;

        assert_close(
            squeezed,
            placement(0.5 - Placement::MIN_SIZE, 0.2, Placement::MIN_SIZE, 0.4),
        );
    }

    #[test]
    fn a_lyrics_area_is_moved_and_resized_inside_the_frame() {
        let area = placement(0.0, 0.5, 1.0, 0.5); // the lower half
        let random = SubtitlePosition::Random { area };
        let top_edge = Grab {
            left: false,
            top: true,
            right: false,
            bottom: false,
        };
        // Its top edge pulled down: only the bottom quarter is left.
        let smaller = moved_position(random, top_edge, area, placement(0.0, 0.75, 1.0, 0.25));
        assert_eq!(
            smaller,
            SubtitlePosition::Random {
                area: placement(0.0, 0.75, 1.0, 0.25)
            }
        );

        // Moved by one of its lines: the whole area moves, but not out of
        // the frame.
        let line = placement(0.4, 0.8, 0.2, 0.05);
        let moved = moved_position(random, Grab::MOVE, line, placement(0.4, 0.9, 0.2, 0.05));
        assert_eq!(
            moved,
            SubtitlePosition::Random {
                area: placement(0.0, 0.5, 1.0, 0.5)
            }
        );
        let up = moved_position(random, Grab::MOVE, line, placement(0.4, 0.6, 0.2, 0.05));
        let SubtitlePosition::Random { area: up } = up else {
            panic!("still random");
        };
        assert!((up.y - 0.3).abs() < 1e-9 && up.height == 0.5);

        // A fixed point follows the line that was dragged.
        let fixed = SubtitlePosition::Fixed { x: 0.5, y: 0.86 };
        let SubtitlePosition::Fixed { x, y } =
            moved_position(fixed, Grab::MOVE, line, placement(0.3, 0.7, 0.2, 0.05))
        else {
            panic!("still fixed");
        };
        assert!((x - 0.4).abs() < 1e-9 && (y - 0.76).abs() < 1e-9);
    }

    #[test]
    fn selected_elements_move_together_but_resize_alone() {
        let ids: Vec<ClipId> = serde_json::from_str("[1, 2, 3]").unwrap();
        let start = placement(0.1, 0.1, 0.2, 0.2);
        let mut drag = Drag {
            clip: ids[0],
            grab: Grab::MOVE,
            start,
            offset: egui::Vec2::ZERO,
            current: placement(0.3, 0.05, 0.2, 0.2),
            start_rotation: 0.0,
            rotation: 0.0,
            start_pointer: egui::Pos2::ZERO,
            snapped: (None, None),
        };
        let group = GroupMove::of(Some(drag), &ids);
        assert_eq!(group.clips, vec![ids[1], ids[2]]);
        assert_close(
            group.moved(placement(0.5, 0.5, 0.1, 0.1)),
            placement(0.7, 0.45, 0.1, 0.1),
        );

        // A dragged element that is not selected moves alone.
        assert!(GroupMove::of(Some(drag), &ids[1..]).clips.is_empty());
        // So does one that is resized.
        drag.grab.left = false;
        assert!(GroupMove::of(Some(drag), &ids).clips.is_empty());
    }

    #[test]
    fn placement_maps_onto_the_frame_on_screen() {
        let rect = to_screen(placement(0.5, 0.0, 0.5, 1.0), frame());

        assert_eq!(rect.min, egui::pos2(600.0, 50.0));
        assert_eq!(rect.max, egui::pos2(1100.0, 550.0));
    }
}
