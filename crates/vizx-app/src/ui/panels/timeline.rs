use eframe::egui::{self, Align2, Color32, FontId, Rect, Stroke};

use crate::theme;

/// Example tracks shown until the project model contains real tracks.
/// Each entry is a track name and the color of its marker.
const PLACEHOLDER_TRACKS: [(&str, Color32); 6] = [
    ("Vocals", Color32::from_rgb(214, 96, 122)),
    ("Music", Color32::from_rgb(86, 148, 222)),
    ("Beat", Color32::from_rgb(226, 160, 66)),
    ("Visualizer", Color32::from_rgb(0, 168, 150)),
    ("Animation", Color32::from_rgb(150, 110, 220)),
    ("Lyrics", Color32::from_rgb(120, 190, 96)),
];

const RULER_HEIGHT: f32 = 22.0;
const TRACK_HEIGHT: f32 = 28.0;
const TRACK_HEADER_WIDTH: f32 = 140.0;
/// Fixed horizontal scale of the placeholder ruler.
const PIXELS_PER_SECOND: f32 = 24.0;
/// Distance between labelled ruler marks, in seconds.
const RULER_LABEL_INTERVAL_SECONDS: u32 = 5;

/// Draws a static timeline: a time ruler and one empty lane per track.
///
/// Nothing here is editable. Clips, a playhead, zooming and scrolling come
/// with the timeline engine in a later milestone.
pub fn show(ui: &mut egui::Ui) {
    let panel_rect = ui.available_rect_before_wrap();
    let painter = ui.painter_at(panel_rect);
    let separator_stroke = Stroke::new(1.0, theme::SEPARATOR);

    painter.rect_filled(panel_rect, 0.0, theme::CANVAS_BACKGROUND);

    let lanes_left = panel_rect.left() + TRACK_HEADER_WIDTH;

    // --- Ruler ---
    let ruler_bottom = panel_rect.top() + RULER_HEIGHT;
    let ruler_rect = Rect::from_min_max(
        panel_rect.left_top(),
        egui::pos2(panel_rect.right(), ruler_bottom),
    );
    painter.rect_filled(ruler_rect, 0.0, theme::PANEL_BACKGROUND);

    let visible_seconds = ((panel_rect.right() - lanes_left) / PIXELS_PER_SECOND).max(0.0) as u32;
    for second in 0..=visible_seconds {
        let x = lanes_left + second as f32 * PIXELS_PER_SECOND;
        let is_labelled = second % RULER_LABEL_INTERVAL_SECONDS == 0;
        let tick_height = if is_labelled { 8.0 } else { 4.0 };

        painter.vline(
            x,
            (ruler_bottom - tick_height)..=ruler_bottom,
            separator_stroke,
        );
        if is_labelled {
            painter.text(
                egui::pos2(x + 3.0, ruler_rect.top() + 2.0),
                Align2::LEFT_TOP,
                format!("{}:{:02}", second / 60, second % 60),
                FontId::monospace(10.0),
                theme::TEXT_MUTED,
            );
        }
    }

    // --- Tracks ---
    for (index, (name, color)) in PLACEHOLDER_TRACKS.iter().enumerate() {
        let top = ruler_bottom + index as f32 * TRACK_HEIGHT;
        let header_rect = Rect::from_min_max(
            egui::pos2(panel_rect.left(), top),
            egui::pos2(lanes_left, top + TRACK_HEIGHT),
        );

        painter.rect_filled(header_rect, 0.0, theme::PANEL_BACKGROUND);
        // Colored marker on the left edge identifies the track.
        painter.rect_filled(
            Rect::from_min_size(header_rect.left_top(), egui::vec2(3.0, TRACK_HEIGHT)),
            0.0,
            *color,
        );
        painter.text(
            header_rect.left_center() + egui::vec2(12.0, 0.0),
            Align2::LEFT_CENTER,
            *name,
            FontId::proportional(12.0),
            ui.visuals().text_color(),
        );
        painter.hline(panel_rect.x_range(), top + TRACK_HEIGHT, separator_stroke);
    }

    // Line between the track headers and the lanes, and under the ruler.
    painter.vline(lanes_left, panel_rect.y_range(), separator_stroke);
    painter.hline(panel_rect.x_range(), ruler_bottom, separator_stroke);

    ui.allocate_rect(panel_rect, egui::Sense::hover());
}
