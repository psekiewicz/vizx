//! The export window: settings before an export, progress while it runs.

use eframe::egui;
use vizx_core::{ExportQuality, ExportSettings, VideoFormat};

use crate::app::{AppCommand, VizxApp};
use crate::theme;

pub fn show(ctx: &egui::Context, app: &mut VizxApp) {
    if app.is_export_settings_open {
        show_settings(ctx, app);
    } else {
        show_progress(ctx, app);
    }
}

/// Format, quality, size and range, then "Export...".
fn show_settings(ctx: &egui::Context, app: &mut VizxApp) {
    let project = &app.session.project().settings;
    let (project_width, project_height) = (project.width, project.height);
    let frames_per_second = project.frame_rate.as_f64();
    let project_seconds = project.duration_seconds();
    let loop_region = app.session.loop_region();

    let mut is_open = true;
    let mut wants_export = false;
    egui::Window::new("Export Video")
        .open(&mut is_open)
        .collapsible(false)
        .resizable(false)
        .default_pos(ctx.content_rect().center() - egui::vec2(180.0, 140.0))
        .show(ctx, |ui| {
            let settings = &mut app.settings.export;
            egui::Grid::new("export_settings")
                .num_columns(2)
                .spacing([12.0, 8.0])
                .show(ui, |ui| {
                    ui.label("Format");
                    egui::ComboBox::from_id_salt("export_format")
                        .width(200.0)
                        .selected_text(settings.format.label())
                        .show_ui(ui, |ui| {
                            for format in VideoFormat::ALL {
                                ui.selectable_value(&mut settings.format, format, format.label());
                            }
                        });
                    ui.end_row();

                    ui.label("Quality");
                    ui.horizontal(|ui| {
                        for quality in ExportQuality::ALL {
                            ui.selectable_value(&mut settings.quality, quality, quality.label());
                        }
                    });
                    ui.end_row();

                    ui.label("Resolution");
                    let size_label = |height: Option<u32>| {
                        let (width, height) = ExportSettings {
                            height,
                            ..ExportSettings::default()
                        }
                        .video_size(project_width, project_height);
                        match height {
                            _ if height == project_height => {
                                format!("{width} x {height} (project)")
                            }
                            _ => format!("{width} x {height}"),
                        }
                    };
                    egui::ComboBox::from_id_salt("export_size")
                        .width(200.0)
                        .selected_text(size_label(settings.height))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut settings.height, None, size_label(None));
                            for height in ExportSettings::HEIGHTS {
                                if height != project_height {
                                    ui.selectable_value(
                                        &mut settings.height,
                                        Some(height),
                                        size_label(Some(height)),
                                    );
                                }
                            }
                        });
                    ui.end_row();

                    ui.label("Range");
                    ui.vertical(|ui| {
                        ui.radio_value(
                            &mut settings.range,
                            None,
                            format!("Whole project ({})", seconds_text(project_seconds)),
                        );
                        let loop_label = match loop_region {
                            Some((start, end)) => format!(
                                "Loop range ({} - {})",
                                seconds_text(start),
                                seconds_text(end)
                            ),
                            None => "Loop range (none set)".to_owned(),
                        };
                        ui.add_enabled_ui(loop_region.is_some(), |ui| {
                            if ui.radio(settings.range.is_some(), loop_label).clicked() {
                                settings.range = loop_region;
                            }
                        });
                    });
                    ui.end_row();
                });

            let (start, end) = settings.range.unwrap_or((0.0, project_seconds));
            let frames = ((end - start).max(0.0) * frames_per_second).ceil();
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!(
                    "{} frames at {frames_per_second:.2} fps",
                    frames as u64
                ))
                .small()
                .color(theme::TEXT_MUTED),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                wants_export = ui.button("Export...").clicked();
            });
        });

    if wants_export {
        app.request(AppCommand::ConfirmExport);
    } else if !is_open {
        app.is_export_settings_open = false;
    }
}

/// `m:ss` for a number of seconds.
fn seconds_text(seconds: f64) -> String {
    let whole = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// Shows the progress of the export, if there is one, and lets the user
/// cancel it. When the export has ended, the window says how it went and
/// stays until it is closed.
fn show_progress(ctx: &egui::Context, app: &mut VizxApp) {
    let Some(export) = app.export() else {
        return;
    };
    let finished = export.finished_frame_count();
    let total = export.frame_count().max(1);
    let result = export.result();
    let output = export.output().display().to_string();

    let mut wants_cancel = false;
    let mut wants_close = false;
    egui::Window::new("Export Video")
        .collapsible(false)
        .resizable(false)
        .default_pos(ctx.content_rect().center() - egui::vec2(210.0, 80.0))
        .show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(
                egui::RichText::new(&output)
                    .small()
                    .color(theme::TEXT_MUTED),
            );
            ui.add_space(6.0);

            match &result {
                None => {
                    ui.add(
                        egui::ProgressBar::new(finished as f32 / total as f32)
                            .text(format!("Frame {finished} of {total}")),
                    );
                    ui.add_space(6.0);
                    if ui.button("Cancel").clicked() {
                        wants_cancel = true;
                    }
                }
                Some(Ok(())) => {
                    ui.label(format!("Done ({total} frames)"));
                    ui.add_space(6.0);
                    wants_close = ui.button("Close").clicked();
                }
                Some(Err(problem)) => {
                    ui.colored_label(ui.visuals().error_fg_color, "Export failed");
                    ui.label(problem);
                    ui.add_space(6.0);
                    wants_close = ui.button("Close").clicked();
                }
            }
        });

    if wants_cancel && let Some(export) = app.export() {
        export.cancel();
    }
    if wants_close {
        app.close_export();
    }
}
