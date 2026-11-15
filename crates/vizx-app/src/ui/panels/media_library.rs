use eframe::egui;
use vizx_core::EditorSession;

use crate::theme;
use crate::ui::format_duration;

/// Categories that have no content yet in this prototype.
const EMPTY_CATEGORIES: [&str; 1] = ["Effects"];

/// Lists the files the project uses. The list is read from the project, so
/// it always matches what is on the timeline and what gets saved.
pub fn show(ui: &mut egui::Ui, session: &EditorSession) {
    let media = session.project().media();

    egui::CollapsingHeader::new("Audio Sources")
        .default_open(true)
        .show(ui, |ui| {
            let mut is_empty = true;
            for source in media {
                let Some(audio) = source.audio() else {
                    continue;
                };
                is_empty = false;
                ui.label(source.file_name()).on_hover_text(&source.path);
                ui.label(
                    egui::RichText::new(format!(
                        "{}  |  {} Hz  |  {}",
                        format_duration(audio.duration_milliseconds),
                        audio.sample_rate,
                        channel_description(audio.channel_count)
                    ))
                    .small()
                    .color(theme::TEXT_MUTED),
                );
                ui.add_space(2.0);
            }
            if is_empty {
                empty_hint(ui);
            }
        });

    egui::CollapsingHeader::new("Visualizers")
        .default_open(true)
        .show(ui, |ui| {
            let mut is_empty = true;
            for source in media.iter().filter(|source| source.is_visualizer()) {
                is_empty = false;
                ui.label(source.file_name()).on_hover_text(&source.path);
                ui.label(
                    egui::RichText::new(if source.is_milkdrop_preset() {
                        "MilkDrop preset"
                    } else {
                        "Visualizer plugin"
                    })
                    .small()
                    .color(theme::TEXT_MUTED),
                );
                ui.add_space(2.0);
            }
            if is_empty {
                empty_hint(ui);
            }
        });

    egui::CollapsingHeader::new("Images")
        .default_open(true)
        .show(ui, |ui| {
            let mut is_empty = true;
            for source in media {
                let Some(image) = source.image() else {
                    continue;
                };
                is_empty = false;
                ui.label(source.file_name()).on_hover_text(&source.path);
                ui.label(
                    egui::RichText::new(format!("{} x {}", image.width, image.height))
                        .small()
                        .color(theme::TEXT_MUTED),
                );
                ui.add_space(2.0);
            }
            if is_empty {
                empty_hint(ui);
            }
        });

    egui::CollapsingHeader::new("Animations")
        .default_open(true)
        .show(ui, |ui| {
            let mut is_empty = true;
            for source in media {
                if let Some(animation) = source.animation() {
                    is_empty = false;
                    ui.label(source.file_name()).on_hover_text(&source.path);
                    ui.label(
                        egui::RichText::new(format!(
                            "{}  |  {} frames",
                            match animation.layout {
                                vizx_project::FrameLayout::Sequence { .. } => "Image sequence",
                                vizx_project::FrameLayout::SpriteSheet { .. } => "Sprite sheet",
                            },
                            animation.frame_count
                        ))
                        .small()
                        .color(theme::TEXT_MUTED),
                    );
                    ui.add_space(2.0);
                    continue;
                }
                let Some(video) = source.video() else {
                    continue;
                };
                is_empty = false;
                ui.label(source.file_name()).on_hover_text(&source.path);
                ui.label(
                    egui::RichText::new(format!(
                        "{}  |  {} x {}",
                        if source.path.to_lowercase().ends_with(".gif") {
                            "Animated GIF"
                        } else {
                            "Video"
                        },
                        video.width,
                        video.height
                    ))
                    .small()
                    .color(theme::TEXT_MUTED),
                );
                ui.add_space(2.0);
            }
            if is_empty {
                empty_hint(ui);
            }
        });

    for category in EMPTY_CATEGORIES {
        egui::CollapsingHeader::new(category)
            .default_open(true)
            .show(ui, empty_hint);
    }
}

fn empty_hint(ui: &mut egui::Ui) {
    ui.label(
        egui::RichText::new("No items")
            .small()
            .color(theme::TEXT_MUTED),
    );
}

fn channel_description(channel_count: u16) -> String {
    match channel_count {
        1 => "mono".to_owned(),
        2 => "stereo".to_owned(),
        count => format!("{count} channels"),
    }
}
