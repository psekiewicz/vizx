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
        let layer_rect = to_screen(placement, composition_rect);
        // The clip's frame and handles stay where its placement is, even
        // while its animation moves the picture, so they do not jump
        // around while the music plays.
        draw_layer_state(&layer_painter, layer_rect, &layer.state);

        let response = ui.interact(
            layer_rect.intersect(panel_rect),
            ui.id().with(("preview_layer", layer.clip_id)),
            egui::Sense::click_and_drag(),
        );
        if response.clicked() || response.drag_started() {
            session.select(Selection::Track(layer.track_id));
        }
        let response = response.on_hover_cursor(CursorIcon::Move);
        follow_drag(
            &response,
            layer.clip_id,
            Grab::MOVE,
            placement,
            composition_rect,
            &mut drag,
            &mut finished_drag,
        );
    }

    // Frame and handles of the selected layer, above every picture.
    let selection = session.selection();
    for layer in layers {
        if selection != Selection::Track(layer.track_id) {
            continue;
        }
        let placement = shown_placement(layer, drag);
        let layer_rect = to_screen(placement, composition_rect);
        painter.rect_stroke(
            layer_rect,
            0.0,
            Stroke::new(1.0, theme::ACCENT),
            StrokeKind::Outside,
        );

        for (grab, position, cursor) in handles(layer_rect) {
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
                placement,
                composition_rect,
                &mut drag,
                &mut finished_drag,
            );
        }
    }

    // While dragging, the picture itself follows the mouse too.
    visual_preview::set_dragged_placement(ui.ctx(), drag.map(|drag| (drag.clip, drag.current)));
    ui.data_mut(|data| match drag {
        Some(drag) => {
            data.insert_temp(drag_id, drag);
        }
        None => data.remove::<Drag>(drag_id),
    });
    if let Some(finished) = finished_drag {
        session.edit_project(|project| project.set_clip_placement(finished.clip, finished.current));
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
}

/// The placement to draw a layer with: the one being dragged, if it is
/// being dragged, otherwise the one stored in the project.
fn shown_placement(layer: &LayerView, drag: Option<Drag>) -> Placement {
    match drag {
        Some(drag) if drag.clip == layer.clip_id => drag.current,
        _ => layer.placement,
    }
}

/// Starts, continues or ends a drag according to what happened to
/// `response` this frame.
fn follow_drag(
    response: &egui::Response,
    clip: ClipId,
    grab: Grab,
    placement: Placement,
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
        });
    }
    let is_this_drag = |drag: &Drag| drag.clip == clip && drag.grab == grab;

    if response.dragged()
        && let Some(active) = drag.as_mut().filter(|active| is_this_drag(active))
    {
        active.offset += response.drag_delta();
        active.current = dragged_placement(active.start, grab, active.offset, composition_rect);
    }
    if response.drag_stopped() && drag.as_ref().is_some_and(is_this_drag) {
        *finished_drag = drag.take();
    }
}

/// The placement that results from dragging the grabbed edges of `start`
/// by `offset` screen points.
fn dragged_placement(
    start: Placement,
    grab: Grab,
    offset: egui::Vec2,
    composition_rect: Rect,
) -> Placement {
    if composition_rect.width() <= 0.0 || composition_rect.height() <= 0.0 {
        return start;
    }
    // Screen points to fractions of the composition.
    let delta_x = f64::from(offset.x / composition_rect.width());
    let delta_y = f64::from(offset.y / composition_rect.height());
    let snap_x = f64::from(SNAP_DISTANCE / composition_rect.width());
    let snap_y = f64::from(SNAP_DISTANCE / composition_rect.height());

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
    fn moving_keeps_the_size() {
        let start = placement(0.2, 0.2, 0.3, 0.4);

        let moved = dragged_placement(start, Grab::MOVE, egui::vec2(100.0, -50.0), frame());

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

        let resized = dragged_placement(start, bottom_right, egui::vec2(100.0, 50.0), frame());

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
        let nudged = dragged_placement(start, right_edge, egui::vec2(4.0, 0.0), frame());
        assert_close(nudged, start);

        // Moving left by 197 points puts the left edge 3 points from the
        // frame edge: it snaps onto it, and the size is kept.
        let moved = dragged_placement(start, Grab::MOVE, egui::vec2(-197.0, 0.0), frame());
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

        let squeezed = dragged_placement(start, left_edge, egui::vec2(900.0, 0.0), frame());

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
