use eframe::egui::{self, Align2, Color32, FontId, Rect, Stroke, StrokeKind};
use vizx_core::EditorSession;

use crate::app::AppCommand;
use crate::milkdrop_preview::MilkdropView;
use crate::theme;

/// Empty space kept between the composition frame and the panel edges.
const FRAME_MARGIN: f32 = 24.0;

/// Draws the composition at the playhead.
///
/// The frame has the shape of the project's composition size. So far the
/// only thing a composition can contain is a MilkDrop visualizer; its
/// latest picture is stretched to fill the frame.
pub fn show(
    ui: &mut egui::Ui,
    session: &EditorSession,
    requested_command: &mut Option<AppCommand>,
    milkdrop: &MilkdropView<'_>,
) {
    let settings = &session.project().settings;
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

    // A message in the middle of the frame, when there is no picture.
    let message = match milkdrop {
        MilkdropView::Picture(texture) => {
            // The whole texture (0,0 to 1,1) drawn over the whole frame.
            let whole_texture = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(
                texture.id(),
                composition_rect,
                whole_texture,
                Color32::WHITE,
            );
            None
        }
        MilkdropView::NotActive => Some("Nothing to show at this time".to_owned()),
        MilkdropView::Rendering { finished, total } if *total > 0 => Some(format!(
            "Pre-rendering MilkDrop: {}%\n({finished} of {total} frames)",
            u64::from(*finished) * 100 / u64::from(*total)
        )),
        MilkdropView::Rendering { .. } => Some("Waiting to pre-render MilkDrop...".to_owned()),
        MilkdropView::WaitingForAudio => {
            Some("Loading audio before pre-rendering MilkDrop...".to_owned())
        }
        MilkdropView::Failed(reason) => Some(reason.clone()),
    };
    if let Some(message) = message {
        let galley = painter.layout(
            message,
            FontId::proportional(13.0),
            theme::TEXT_MUTED,
            (composition_rect.width() - 24.0).max(40.0),
        );
        painter.galley(
            composition_rect.center() - galley.size() / 2.0,
            galley,
            theme::TEXT_MUTED,
        );
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

    // Claim the whole panel and offer a context menu on right-click.
    let response = ui.allocate_rect(panel_rect, egui::Sense::click());
    response.context_menu(|ui| {
        if ui.button("Project Settings...").clicked() {
            *requested_command = Some(AppCommand::OpenProjectSettings);
        }
    });
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
