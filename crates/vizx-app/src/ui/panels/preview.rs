use eframe::egui::{self, Align2, Color32, CursorIcon, FontId, Rect, Stroke, StrokeKind};
use vizx_core::{EditorSession, Selection};
use vizx_project::{ClipId, Placement};

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
    let background = ui.interact(
        panel_rect,
        ui.id().with("preview_background"),
        egui::Sense::click(),
    );
    if background.clicked() {
        session.select(Selection::Nothing);
    }
    background.context_menu(|ui| {
        if ui.button("Project Settings...").clicked() {
            *requested_command = Some(AppCommand::OpenProjectSettings);
        }
    });

    if let Some(problem) = preview.composition_error() {
        draw_message(
            &painter,
            composition_rect,
            format!("The picture could not be drawn: {problem}"),
        );
    } else if layers.is_empty() {
        draw_message(
            &painter,
            composition_rect,
            "Nothing to show at this time".to_owned(),
        );
    }

    // A move or resize in progress. It lives in egui's temporary storage
    // between frames and is written to the project when the mouse button is
    // released, so the project (and the pre-render that depends on the
    // size) changes once per drag, not once per mouse movement.
    let drag_id = ui.id().with("preview_drag");
    let mut drag: Option<Drag> = ui.data(|data| data.get_temp(drag_id));
    let mut finished_drag: Option<Drag> = None;

    // Only the part of a visualizer inside the frame is visible.
    let layer_painter = ui.painter_at(composition_rect);
    for layer in layers {
        let placement = shown_placement(layer, drag);
        let rotation = shown_rotation(layer, drag);
        let layer_rect = to_screen(placement, composition_rect);
        // The clip's frame and handles stay where its placement is, even
        // while its animation moves the picture, so they do not jump
        // around while the music plays.
        draw_layer_state(&layer_painter, layer_rect, &layer.state);

        // A turned element takes the mouse only inside its own outline, so
        // the corners of the box around it do not hide the layers below.
        let corners = turned_corners(layer_rect, rotation);
        let reach = Rect::from_points(&corners);
        let id = ui.id().with(("preview_layer", layer.clip_id));
        let is_over_it = ui
            .ctx()
            .pointer_latest_pos()
            .is_some_and(|pointer| is_inside_turned(layer_rect, rotation, pointer));
        let is_held = ui.ctx().is_being_dragged(id) || drag.is_some_and(|drag| drag.clip == layer.clip_id);
        let sense = if is_over_it || is_held {
            egui::Sense::click_and_drag()
        } else {
            egui::Sense::hover()
        };
        let response = ui.interact(reach.intersect(panel_rect), id, sense);
        if response.clicked() || response.drag_started() {
            session.select(Selection::Clip(layer.clip_id));
        }
        let response = response.on_hover_cursor(CursorIcon::Move);
        follow_drag(
            &response,
            layer.clip_id,
            Grab::MOVE,
            (placement, rotation),
            composition_rect,
            &mut drag,
            &mut finished_drag,
        );
    }

    // Frame and handles of the selected layer, above every picture.
    for layer in layers {
        if !session.is_clip_selected(layer.track_id, layer.clip_id) {
            continue;
        }
        let placement = shown_placement(layer, drag);
        let rotation = shown_rotation(layer, drag);
        let layer_rect = to_screen(placement, composition_rect);
        let center = layer_rect.center();
        let corners = turned_corners(layer_rect, rotation);
        painter.add(egui::Shape::closed_line(
            corners.to_vec(),
            Stroke::new(1.0, theme::ACCENT),
        ));

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
            .on_hover_text("Drag to turn. Hold Shift for steps of 15\u{b0}.");
        follow_drag(
            &response,
            layer.clip_id,
            Grab::ROTATE,
            (placement, rotation),
            composition_rect,
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
                composition_rect,
                &mut drag,
                &mut finished_drag,
            );
        }
    }

    // While dragging, the picture itself follows the mouse too.
    visual_preview::set_dragged_placement(
        ui.ctx(),
        drag.map(|drag| (drag.clip, drag.current, drag.rotation)),
    );
    ui.data_mut(|data| match drag {
        Some(drag) => {
            data.insert_temp(drag_id, drag);
        }
        None => data.remove::<Drag>(drag_id),
    });
    if let Some(finished) = finished_drag {
        // Changes the clip at the playhead: its resting place, or the
        // keyframe there (a new one if the clip has keyframes elsewhere).
        let playhead_milliseconds = (session.playhead_seconds() * 1000.0).round() as u64;
        session.edit_project(|project| {
            project.edit_clip_transform_at(finished.clip, playhead_milliseconds, |transform| {
                transform.placement = finished.current;
                transform.rotation_degrees = finished.rotation;
            })
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
}

/// The placement to draw a layer with: the one being dragged, if it is
/// being dragged, otherwise the one stored in the project.
fn shown_placement(layer: &LayerView, drag: Option<Drag>) -> Placement {
    match drag {
        Some(drag) if drag.clip == layer.clip_id => drag.current,
        _ => layer.placement,
    }
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
    composition_rect: Rect,
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
            active.current = turned_dragged_placement(
                active.start,
                active.start_rotation,
                grab,
                active.offset,
                composition_rect,
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
) -> Placement {
    let is_turned = rotation.rem_euclid(360.0).abs() > 1e-9;
    if grab == Grab::MOVE || !is_turned {
        // Guides along the frame's edges only make sense for an upright
        // element.
        return dragged_placement(start, grab, offset, composition_rect, !is_turned);
    }
    let local_offset = turned(offset, -rotation);
    let upright = dragged_placement(start, grab, local_offset, composition_rect, false);

    // The upright result has its middle moved along the screen's axes;
    // turn that move to follow the element's own axes.
    let start_center = to_screen(start, composition_rect).center();
    let moved = to_screen(upright, composition_rect).center() - start_center;
    let center = start_center + turned(moved, rotation);
    Placement {
        x: f64::from((center.x - composition_rect.left()) / composition_rect.width())
            - upright.width / 2.0,
        y: f64::from((center.y - composition_rect.top()) / composition_rect.height())
            - upright.height / 2.0,
        ..upright
    }
    .sanitized()
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

/// The placement that results from dragging the grabbed edges of `start`
/// by `offset` screen points. With `snap`, edges jump onto the frame's
/// edges and middle when they come close.
fn dragged_placement(
    start: Placement,
    grab: Grab,
    offset: egui::Vec2,
    composition_rect: Rect,
    snap: bool,
) -> Placement {
    if composition_rect.width() <= 0.0 || composition_rect.height() <= 0.0 {
        return start;
    }
    // Screen points to fractions of the composition.
    let delta_x = f64::from(offset.x / composition_rect.width());
    let delta_y = f64::from(offset.y / composition_rect.height());
    let snap_distance = if snap { SNAP_DISTANCE } else { 0.0 };
    let snap_x = f64::from(snap_distance / composition_rect.width());
    let snap_y = f64::from(snap_distance / composition_rect.height());

    let (left, right) = dragged_axis(
        start.x,
        start.right(),
        (grab.left, grab.right),
        delta_x,
        snap_x,
    );
    let (top, bottom) = dragged_axis(
        start.y,
        start.bottom(),
        (grab.top, grab.bottom),
        delta_y,
        snap_y,
    );
    Placement {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
    .sanitized()
}

/// Moves the grabbed ends of one axis (`low`..`high`) by `delta`, snapping
/// to the guides at 0, 0.5 and 1 and keeping the minimum size.
fn dragged_axis(
    low: f64,
    high: f64,
    (moves_low, moves_high): (bool, bool),
    delta: f64,
    snap_distance: f64,
) -> (f64, f64) {
    let snap = |value: f64| {
        [0.0, 0.5, 1.0]
            .into_iter()
            .find(|guide| (value - guide).abs() <= snap_distance)
    };

    match (moves_low, moves_high) {
        // Moving: the size stays; whichever end reaches a guide first wins.
        (true, true) => {
            let size = high - low;
            let (moved_low, moved_high) = (low + delta, high + delta);
            if let Some(guide) = snap(moved_low) {
                (guide, guide + size)
            } else if let Some(guide) = snap(moved_high) {
                (guide - size, guide)
            } else {
                (moved_low, moved_high)
            }
        }
        (true, false) => {
            let moved = snap(low + delta).unwrap_or(low + delta);
            (moved.min(high - Placement::MIN_SIZE), high)
        }
        (false, true) => {
            let moved = snap(high + delta).unwrap_or(high + delta);
            (low, moved.max(low + Placement::MIN_SIZE))
        }
        (false, false) => (low, high),
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
        LayerState::Rendering { .. } => "Waiting to pre-render...".to_owned(),
        LayerState::WaitingForAudio => "Loading audio before pre-rendering...".to_owned(),
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
        let grown =
            turned_dragged_placement(start, 90.0, right_edge, egui::vec2(0.0, 50.0), frame());
        assert_close(grown, placement(0.375, 0.45, 0.25, 0.2));

        // Moving the mouse sideways does not resize it.
        let sideways =
            turned_dragged_placement(start, 90.0, right_edge, egui::vec2(50.0, 0.0), frame());
        assert_close(sideways, start);

        // Upright, it is the ordinary resize.
        let upright =
            turned_dragged_placement(start, 0.0, right_edge, egui::vec2(50.0, 0.0), frame());
        assert_close(upright, placement(0.4, 0.4, 0.25, 0.2));
    }

    #[test]
    fn moving_keeps_the_size() {
        let start = placement(0.2, 0.2, 0.3, 0.4);

        let moved = dragged_placement(start, Grab::MOVE, egui::vec2(100.0, -50.0), frame(), true);

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

        let resized =
            dragged_placement(start, bottom_right, egui::vec2(100.0, 50.0), frame(), true);

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
        let nudged = dragged_placement(start, right_edge, egui::vec2(4.0, 0.0), frame(), true);
        assert_close(nudged, start);

        // Moving left by 197 points puts the left edge 3 points from the
        // frame edge: it snaps onto it, and the size is kept.
        let moved = dragged_placement(start, Grab::MOVE, egui::vec2(-197.0, 0.0), frame(), true);
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

        let squeezed = dragged_placement(start, left_edge, egui::vec2(900.0, 0.0), frame(), true);

        assert_close(
            squeezed,
            placement(0.5 - Placement::MIN_SIZE, 0.2, Placement::MIN_SIZE, 0.4),
        );
    }

    #[test]
    fn placement_maps_onto_the_frame_on_screen() {
        let rect = to_screen(placement(0.5, 0.0, 0.5, 1.0), frame());

        assert_eq!(rect.min, egui::pos2(600.0, 50.0));
        assert_eq!(rect.max, egui::pos2(1100.0, 550.0));
    }
}
