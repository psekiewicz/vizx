use eframe::egui;
use vizx_core::EditorSession;
use vizx_project::MediaContent;

use crate::app::AppCommand;
use crate::theme;
use crate::ui::format_duration;

/// Categories that have no content yet in this prototype.
const EMPTY_CATEGORIES: [&str; 3] = ["Images", "Animations", "Effects"];

/// Lists the files the project uses. The list is read from the project, so
/// it always matches what is on the timeline and what gets saved.
pub fn show(
    ui: &mut egui::Ui,
    session: &EditorSession,
    requested_command: &mut Option<AppCommand>,
) {
    if ui.button("Import Audio...").clicked() {
        *requested_command = Some(AppCommand::ImportAudio);
    }
    ui.add_space(4.0);

    egui::CollapsingHeader::new("Audio Sources")
        .default_open(true)
        .show(ui, |ui| {
            let media = session.project().media();
            if media.is_empty() {
                empty_hint(ui);
            }
            for source in media {
                let MediaContent::Audio(audio) = &source.content;
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
