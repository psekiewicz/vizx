//! Visual style of the application: a dark, compact desktop-editor look.
//!
//! All colors and spacing live here so individual panels never hardcode
//! their own look and the interface stays consistent.

use eframe::egui::{self, Color32, CornerRadius, Stroke};

/// Background of the area behind the composition and the timeline lanes.
pub const CANVAS_BACKGROUND: Color32 = Color32::from_rgb(18, 19, 21);
/// Background of panels and bars.
pub const PANEL_BACKGROUND: Color32 = Color32::from_rgb(34, 35, 39);
/// Background of the tab strip above docked panels.
pub const TAB_BAR_BACKGROUND: Color32 = Color32::from_rgb(25, 26, 29);
/// Thin lines separating regions.
pub const SEPARATOR: Color32 = Color32::from_rgb(52, 54, 60);
/// Accent color for selection and the active state.
pub const ACCENT: Color32 = Color32::from_rgb(0, 168, 150);
/// The playhead line on the timeline.
pub const PLAYHEAD: Color32 = Color32::from_rgb(255, 92, 92);
/// Secondary text such as hints and placeholders.
pub const TEXT_MUTED: Color32 = Color32::from_rgb(128, 132, 142);
/// Text that reports a problem.
pub const ERROR: Color32 = Color32::from_rgb(235, 110, 100);

const CORNER_RADIUS: CornerRadius = CornerRadius::same(2);

/// Installs the VizX style into the egui context. Called once at startup.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);

    ctx.style_mut_of(egui::Theme::Dark, |style| {
        let visuals = &mut style.visuals;
        visuals.panel_fill = PANEL_BACKGROUND;
        visuals.window_fill = PANEL_BACKGROUND;
        visuals.extreme_bg_color = CANVAS_BACKGROUND;
        visuals.faint_bg_color = Color32::from_rgb(40, 41, 46);
        visuals.window_corner_radius = CORNER_RADIUS;
        visuals.menu_corner_radius = CORNER_RADIUS;
        visuals.window_stroke = Stroke::new(1.0, SEPARATOR);
        visuals.selection.bg_fill = ACCENT.gamma_multiply(0.45);
        visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
        visuals.hyperlink_color = ACCENT;

        visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, SEPARATOR);
        for widget in [
            &mut visuals.widgets.noninteractive,
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
            &mut visuals.widgets.open,
        ] {
            widget.corner_radius = CORNER_RADIUS;
        }

        // Compact spacing: more content fits on screen, as in other editors.
        let spacing = &mut style.spacing;
        spacing.item_spacing = egui::vec2(6.0, 4.0);
        spacing.button_padding = egui::vec2(6.0, 2.0);
        spacing.interact_size.y = 20.0;
    });
}

/// Style for the docking system, derived from the egui style above.
pub fn dock_style(egui_style: &egui::Style) -> egui_dock::Style {
    let mut style = egui_dock::Style::from_egui(egui_style);

    style.tab_bar.bg_fill = TAB_BAR_BACKGROUND;
    style.tab_bar.hline_color = SEPARATOR;
    style.tab_bar.corner_radius = CornerRadius::ZERO;
    style.tab.tab_body.bg_fill = PANEL_BACKGROUND;
    style.tab.tab_body.stroke = Stroke::NONE;
    style.tab.tab_body.corner_radius = CornerRadius::ZERO;
    style.separator.color_idle = CANVAS_BACKGROUND;
    style.separator.color_hovered = ACCENT;
    style.separator.color_dragged = ACCENT;
    style.overlay.selection_color = ACCENT.gamma_multiply(0.35);

    for tab_state in [
        &mut style.tab.active,
        &mut style.tab.inactive,
        &mut style.tab.focused,
        &mut style.tab.hovered,
    ] {
        tab_state.corner_radius = CornerRadius::ZERO;
    }
    style.tab.active.bg_fill = PANEL_BACKGROUND;
    style.tab.focused.bg_fill = PANEL_BACKGROUND;
    style.tab.inactive.bg_fill = TAB_BAR_BACKGROUND;
    style.tab.inactive.text_color = TEXT_MUTED;

    style
}
