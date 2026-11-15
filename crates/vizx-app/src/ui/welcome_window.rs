//! The welcome window shown when VizX starts, in the spirit of Blender's
//! splash screen: a banner, ways to start a new project from a template on
//! the left, recent projects on the right. A click outside or Esc closes it.

use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Stroke, Vec2};

use crate::app::{AppCommand, VizxApp};
use crate::theme;

/// A ready-made start for a new project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    /// The usual empty layers.
    Empty,
    /// A song with a spectrum visualizer over the whole picture.
    MusicVisualizer,
    /// A song, then the Lyrics window to turn its words into subtitles.
    LyricsVideo,
    /// A song, a cover picture behind, and a ring visualizer around its
    /// middle.
    CoverArtVideo,
}

impl Template {
    pub const ALL: [Self; 4] = [
        Self::Empty,
        Self::MusicVisualizer,
        Self::LyricsVideo,
        Self::CoverArtVideo,
    ];

    fn label(self) -> &'static str {
        match self {
            Self::Empty => "Empty Project",
            Self::MusicVisualizer => "Music Visualizer",
            Self::LyricsVideo => "Lyrics Video",
            Self::CoverArtVideo => "Cover Art Video",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Empty => "Start from nothing",
            Self::MusicVisualizer => "Pick a song; a spectrum visualizer follows it",
            Self::LyricsVideo => "Pick a song, then paste and time its lyrics",
            Self::CoverArtVideo => "Pick a song and a cover; a ring pulses around it",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Empty => "\u{2b1c}",
            Self::MusicVisualizer => "\u{1f3b5}",
            Self::LyricsVideo => "\u{1f4dd}",
            Self::CoverArtVideo => "\u{1f5bc}",
        }
    }
}

const WIDTH: f32 = 640.0;
const BANNER_HEIGHT: f32 = 190.0;

pub fn show(ctx: &egui::Context, app: &mut VizxApp) {
    if !app.is_welcome_open {
        return;
    }
    let mut chosen = None;
    let response = egui::Modal::new(egui::Id::new("welcome")).show(ctx, |ui| {
        ui.set_width(WIDTH);
        banner(ui);
        ui.add_space(10.0);
        ui.columns(2, |columns| {
            new_project_column(&mut columns[0], &mut chosen);
            recent_column(&mut columns[1], app, &mut chosen);
        });
        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| {
            let mut show = app.settings.show_welcome;
            if ui.checkbox(&mut show, "Show at startup").changed() {
                app.set_show_welcome(show);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(
                        "F1: keyboard shortcuts  |  right-click the timeline to add",
                    )
                    .small()
                    .color(theme::TEXT_MUTED),
                );
            });
        });
    });

    if let Some(command) = chosen {
        app.is_welcome_open = false;
        app.request(command);
    } else if response.should_close() {
        app.is_welcome_open = false;
    }
}

/// The picture at the top: the name over a little audio-reactive-looking
/// spectrum that moves by itself.
fn banner(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(WIDTH, BANNER_HEIGHT), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let radius = CornerRadius::same(6);
    painter.rect_filled(rect, radius, Color32::from_rgb(14, 16, 22));

    // Bars that rise and fall like a spectrum, in the accent colour fading
    // to violet.
    let time = ui.input(|input| input.time) as f32;
    let bar_count = 48;
    let gap = 3.0;
    let bar_width = (rect.width() - gap * (bar_count as f32 + 1.0)) / bar_count as f32;
    for index in 0..bar_count {
        let place = index as f32 / bar_count as f32;
        let wave = (time * 1.7 + place * 9.0).sin() * 0.5 + 0.5;
        let beat = ((time * 2.0).fract() * -6.0).exp() * (1.0 - place);
        let shape = 0.25 + 0.55 * (1.0 - place).powf(0.7) * wave + 0.3 * beat;
        let height = (rect.height() - 20.0) * shape.clamp(0.05, 1.0);
        let x = rect.left() + gap + index as f32 * (bar_width + gap);
        let bar = Rect::from_min_max(
            Pos2::new(x, rect.bottom() - height),
            Pos2::new(x + bar_width, rect.bottom()),
        );
        let color = lerp_color(theme::ACCENT, Color32::from_rgb(140, 90, 230), place);
        painter.rect_filled(bar, CornerRadius::same(2), color.gamma_multiply(0.55));
    }
    // Darker at the top, so the name stands out.
    let shade = Rect::from_min_max(rect.min, Pos2::new(rect.right(), rect.center().y));
    painter.rect_filled(shade, radius, Color32::from_black_alpha(90));

    painter.text(
        rect.left_top() + Vec2::new(24.0, 22.0),
        egui::Align2::LEFT_TOP,
        "VizX",
        FontId::proportional(54.0),
        Color32::WHITE,
    );
    painter.text(
        rect.left_top() + Vec2::new(27.0, 86.0),
        egui::Align2::LEFT_TOP,
        "Music-driven visuals, lyrics and animation",
        FontId::proportional(15.0),
        Color32::from_gray(200),
    );
    painter.text(
        rect.right_bottom() - Vec2::new(12.0, 10.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("v{}", vizx_core::APP_VERSION),
        FontId::proportional(12.0),
        Color32::from_gray(210),
    );
    painter.rect_stroke(
        rect,
        radius,
        Stroke::new(1.0, theme::SEPARATOR),
        egui::StrokeKind::Inside,
    );
    // Keep the bars moving.
    ui.ctx().request_repaint();
}

fn lerp_color(from: Color32, to: Color32, amount: f32) -> Color32 {
    let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * amount) as u8;
    Color32::from_rgb(
        mix(from.r(), to.r()),
        mix(from.g(), to.g()),
        mix(from.b(), to.b()),
    )
}

fn new_project_column(ui: &mut egui::Ui, chosen: &mut Option<AppCommand>) {
    ui.label(egui::RichText::new("New Project").strong());
    ui.add_space(2.0);
    for template in Template::ALL {
        let text = format!("{}  {}", template.icon(), template.label());
        let button = egui::Button::new(egui::RichText::new(text).size(15.0))
            .frame(false)
            .min_size(Vec2::new(ui.available_width(), 0.0));
        if ui
            .add(button)
            .on_hover_text(template.description())
            .clicked()
        {
            *chosen = Some(AppCommand::StartFromTemplate(template));
        }
        ui.label(
            egui::RichText::new(template.description())
                .small()
                .color(theme::TEXT_MUTED),
        );
        ui.add_space(4.0);
    }
}

fn recent_column(ui: &mut egui::Ui, app: &VizxApp, chosen: &mut Option<AppCommand>) {
    ui.label(egui::RichText::new("Recent Projects").strong());
    ui.add_space(2.0);
    let recent = &app.settings.recent_projects;
    if recent.is_empty() {
        ui.label(
            egui::RichText::new("Projects you open or save will be listed here.")
                .small()
                .color(theme::TEXT_MUTED),
        );
    }
    for (index, path) in recent.iter().enumerate().take(8) {
        let name = path.file_stem().map_or_else(
            || path.display().to_string(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        let exists = path.is_file();
        let button = egui::Button::new(format!("\u{1f4c4}  {name}"))
            .frame(false)
            .min_size(Vec2::new(ui.available_width(), 0.0));
        let response = ui
            .add_enabled(exists, button)
            .on_hover_text(path.display().to_string())
            .on_disabled_hover_text("This file no longer exists");
        if response.clicked() {
            *chosen = Some(AppCommand::OpenRecent(index));
        }
    }
    ui.add_space(6.0);
    if ui.button("Open...").clicked() {
        *chosen = Some(AppCommand::OpenProject);
    }
}
