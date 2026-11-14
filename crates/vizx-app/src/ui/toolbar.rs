use eframe::egui;

use crate::app::{AppCommand, VizxApp};
use crate::theme;

pub fn show(ui: &mut egui::Ui, app: &mut VizxApp) {
    ui.horizontal(|ui| {
        command_button(
            ui,
            app,
            "New",
            "New project (Ctrl+N)",
            AppCommand::NewProject,
        );
        command_button(
            ui,
            app,
            "Open",
            "Open project (Ctrl+O)",
            AppCommand::OpenProject,
        );
        command_button(
            ui,
            app,
            "Save",
            "Save project (Ctrl+S)",
            AppCommand::SaveProject,
        );

        ui.separator();

        ui.menu_button("Add", |ui| {
            if let Some(command) = crate::ui::add_menu_items(ui) {
                app.request(command);
            }
        })
        .response
        .on_hover_text("Add audio, a visualizer, a layer...");

        ui.separator();

        // Transport controls. The play button turns into a pause button
        // while playing.
        let (play_symbol, play_tooltip) = if app.is_playing() {
            ("\u{23F8}", "Pause (Space)")
        } else {
            ("\u{25B6}", "Play (Space)")
        };
        command_button(
            ui,
            app,
            "\u{23EE}",
            "Go to start (Home)",
            AppCommand::GoToStart,
        );
        command_button(
            ui,
            app,
            play_symbol,
            play_tooltip,
            AppCommand::TogglePlayback,
        );
        command_button(
            ui,
            app,
            "\u{23F9}",
            "Stop and return to start",
            AppCommand::StopPlayback,
        );
        command_button(ui, app, "\u{23ED}", "Go to end (End)", AppCommand::GoToEnd);

        ui.separator();

        let time_color = if app.is_playing() {
            ui.visuals().strong_text_color()
        } else {
            theme::TEXT_MUTED
        };
        ui.label(
            egui::RichText::new(format_timecode(app.session.playhead_seconds()))
                .monospace()
                .color(time_color),
        )
        .on_hover_text("Playhead position (hours:minutes:seconds.milliseconds)");
    });
}

fn command_button(
    ui: &mut egui::Ui,
    app: &mut VizxApp,
    label: &str,
    tooltip: &str,
    command: AppCommand,
) {
    if ui.button(label).on_hover_text(tooltip).clicked() {
        app.request(command);
    }
}

/// Formats a position as `hh:mm:ss.mmm`.
fn format_timecode(seconds: f64) -> String {
    let total_milliseconds = (seconds.max(0.0) * 1000.0).round() as u64;
    let milliseconds = total_milliseconds % 1000;
    let total_seconds = total_milliseconds / 1000;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        total_seconds / 3600,
        (total_seconds / 60) % 60,
        total_seconds % 60,
        milliseconds
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timecode_shows_hours_minutes_seconds_and_milliseconds() {
        assert_eq!(format_timecode(0.0), "00:00:00.000");
        assert_eq!(format_timecode(65.25), "00:01:05.250");
        assert_eq!(format_timecode(3723.004), "01:02:03.004");
        assert_eq!(format_timecode(-5.0), "00:00:00.000");
    }
}
