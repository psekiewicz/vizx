use eframe::egui;

use crate::app::{AppCommand, VizxApp};

/// What the user chose to do about unsaved changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Save,
    Discard,
    Cancel,
}

/// Shows the confirmation dialog while a command is on hold because the
/// project has unsaved changes. Returns the user's decision in the frame it
/// is made, and `None` otherwise.
///
/// The dialog is modal: the rest of the interface cannot be used until it is
/// answered.
pub fn show(ctx: &egui::Context, app: &VizxApp) -> Option<Decision> {
    let command = app.command_awaiting_confirmation()?;
    let mut decision = None;

    let modal = egui::Modal::new(egui::Id::new("unsaved_changes_dialog")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.heading("Unsaved changes");
        ui.add_space(4.0);
        ui.label(format!(
            "Save changes to \"{}\" before {}?",
            app.session.project().metadata.name,
            describe(command)
        ));
        ui.add_space(12.0);

        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                decision = Some(Decision::Save);
            }
            if ui.button("Don't Save").clicked() {
                decision = Some(Decision::Discard);
            }
            if ui.button("Cancel").clicked() {
                decision = Some(Decision::Cancel);
            }
        });
    });

    // Pressing Escape or clicking outside the dialog counts as Cancel.
    if decision.is_none() && modal.should_close() {
        decision = Some(Decision::Cancel);
    }

    decision
}

fn describe(command: AppCommand) -> &'static str {
    match command {
        AppCommand::NewProject => "creating a new project",
        AppCommand::OpenProject => "opening another project",
        AppCommand::Exit => "exiting",
        AppCommand::SaveProject
        | AppCommand::SaveProjectAs
        | AppCommand::ImportAudio
        | AppCommand::OpenProjectSettings => "continuing",
    }
}
