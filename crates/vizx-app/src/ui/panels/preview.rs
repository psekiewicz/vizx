use eframe::egui::{self, Align2, Color32, FontId, Rect, Stroke, StrokeKind};
use vizx_core::EditorSession;

use crate::theme;

/// Empty space kept between the composition frame and the panel edges.
const FRAME_MARGIN: f32 = 24.0;

/// Draws the composition area.
///
/// There is no renderer yet, so this only shows an empty frame. The frame
/// does use the real composition size from the project settings, so changing
/// the resolution in the Properties panel changes its shape here.
pub fn show(ui: &mut egui::Ui, session: &EditorSession) {
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
    painter.text(
        composition_rect.center(),
        Align2::CENTER_CENTER,
        "Composition preview",
        FontId::proportional(14.0),
        theme::TEXT_MUTED,
    );
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

    // Tell egui this panel used all of its space.
    ui.allocate_rect(panel_rect, egui::Sense::hover());
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
