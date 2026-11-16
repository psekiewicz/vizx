use eframe::egui;

use crate::app::{AppCommand, VizxApp};
use crate::theme;

/// The toolbar holds the transport controls and the time display. Project
/// commands (New, Open, Save) are only in the File menu, and adding content
/// is only in the timeline's context menu.
pub fn show(ui: &mut egui::Ui, app: &mut VizxApp) {
    ui.horizontal(|ui| {
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
            "Stop",
            AppCommand::StopPlayback,
        );
        command_button(ui, app, "\u{23ED}", "Go to end (End)", AppCommand::GoToEnd);
        let looping = app.session.is_looping();
        let loop_tooltip = match app.session.loop_region() {
            Some((start, end)) => format!(
                "Loop (L)\n{} - {}\nSet range: Shift+drag on the ruler, or I / O",
                format_timecode(start),
                format_timecode(end)
            ),
            None => "Loop (L)\nSet range: Shift+drag on the ruler, or I / O".to_owned(),
        };
        if ui
            .selectable_label(looping, "\u{1F501}")
            .on_hover_text(loop_tooltip)
            .clicked()
        {
            app.request(AppCommand::ToggleLoop);
        }

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
        .on_hover_text("hh:mm:ss.ms");

        ui.separator();
        // Playback volume; the export is not affected.
        let mut volume = app.transport.volume() * 100.0;
        let symbol = if volume <= 0.0 { "\u{1F507}" } else { "\u{1F50A}" };
        ui.label(symbol);
        let slider = egui::Slider::new(&mut volume, 0.0..=100.0)
            .show_value(false)
            .trailing_fill(true);
        let response = ui
            .add_sized([90.0, ui.spacing().interact_size.y], slider)
            .on_hover_text(format!("Volume {volume:.0}%"));
        if response.changed() {
            app.transport.set_volume(volume / 100.0);
        }
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
