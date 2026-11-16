//! Offered at startup when VizX did not close properly last time (a crash,
//! a power cut): the autosaved projects can be opened again.

use eframe::egui;

use crate::app::VizxApp;
use crate::theme;

pub fn show(ctx: &egui::Context, app: &mut VizxApp) {
    if app.recoveries.is_empty() {
        return;
    }
    let mut chosen: Option<(usize, bool)> = None;
    // Recovering replaces the open project, so it must not hold new work.
    let can_recover = !app.session.has_unsaved_changes();
    egui::Window::new("Recover Work")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label("VizX didn't shut down properly. Autosaved projects:");
            ui.add_space(6.0);
            egui::Grid::new("recoveries")
                .num_columns(3)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    for (index, recovery) in app.recoveries.iter().enumerate() {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&recovery.project_name).strong());
                            let place = recovery
                                .project_path
                                .as_ref()
                                .map_or("never saved".to_owned(), |path| {
                                    path.display().to_string()
                                });
                            ui.label(
                                egui::RichText::new(format!(
                                    "{}  |  {place}",
                                    autosaved_ago(recovery.saved_at)
                                ))
                                .small()
                                .color(theme::TEXT_MUTED),
                            );
                        });
                        if ui
                            .add_enabled(can_recover, egui::Button::new("Recover"))
                            .on_disabled_hover_text(
                                "Save the current project first (it will be replaced)",
                            )
                            .clicked()
                        {
                            chosen = Some((index, true));
                        }
                        if ui
                            .button("Discard")
                            .on_hover_text("Delete the autosave")
                            .clicked()
                        {
                            chosen = Some((index, false));
                        }
                        ui.end_row();
                    }
                });
        });
    match chosen {
        Some((index, true)) => app.recover(index),
        Some((index, false)) => app.discard_recovery(index),
        None => {}
    }
}

/// "autosaved 5 minutes ago" and the like.
fn autosaved_ago(saved_at: Option<std::time::SystemTime>) -> String {
    let Some(age) = saved_at.and_then(|time| time.elapsed().ok()) else {
        return "autosaved".to_owned();
    };
    let minutes = age.as_secs() / 60;
    match minutes {
        0 => "autosaved less than a minute ago".to_owned(),
        1..=59 => format!("autosaved {minutes} min ago"),
        60..=2879 => format!("autosaved {} h ago", minutes / 60),
        _ => format!("autosaved {} days ago", minutes / 1440),
    }
}
