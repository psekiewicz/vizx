use eframe::egui;

use crate::app::{AppCommand, VizxApp};
use crate::ui::NOT_AVAILABLE_YET;
use crate::ui::workspace::PanelKind;

pub fn show(ui: &mut egui::Ui, app: &mut VizxApp) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| file_menu(ui, app));
        ui.menu_button("Edit", edit_menu);
        ui.menu_button("View", |ui| view_menu(ui, app));
        ui.menu_button("Playback", |ui| playback_menu(ui, app));
        ui.menu_button("Help", |ui| help_menu(ui, app));
    });
}

fn file_menu(ui: &mut egui::Ui, app: &mut VizxApp) {
    command_item(ui, app, "New Project", "Ctrl+N", AppCommand::NewProject);
    command_item(
        ui,
        app,
        "Open Project...",
        "Ctrl+O",
        AppCommand::OpenProject,
    );
    ui.separator();
    command_item(ui, app, "Save", "Ctrl+S", AppCommand::SaveProject);
    command_item(
        ui,
        app,
        "Save As...",
        "Ctrl+Shift+S",
        AppCommand::SaveProjectAs,
    );
    ui.separator();
    command_item(
        ui,
        app,
        "Import Audio...",
        "Ctrl+I",
        AppCommand::ImportAudio,
    );
    ui.separator();
    command_item(
        ui,
        app,
        "Project Settings...",
        "",
        AppCommand::OpenProjectSettings,
    );
    unavailable_item(ui, "Export Video...", "");
    ui.separator();
    command_item(ui, app, "Exit", "", AppCommand::Exit);
}

/// A menu entry that requests an application command when clicked.
fn command_item(
    ui: &mut egui::Ui,
    app: &mut VizxApp,
    label: &str,
    shortcut: &str,
    command: AppCommand,
) {
    if ui
        .add(egui::Button::new(label).shortcut_text(shortcut))
        .clicked()
    {
        app.request(command);
    }
}

fn edit_menu(ui: &mut egui::Ui) {
    unavailable_item(ui, "Undo", "Ctrl+Z");
    unavailable_item(ui, "Redo", "Ctrl+Shift+Z");
    ui.separator();
    unavailable_item(ui, "Cut", "Ctrl+X");
    unavailable_item(ui, "Copy", "Ctrl+C");
    unavailable_item(ui, "Paste", "Ctrl+V");
    unavailable_item(ui, "Delete", "Del");
}

fn view_menu(ui: &mut egui::Ui, app: &mut VizxApp) {
    // One checkbox per dockable panel: unchecking closes it, checking brings
    // it back. This is how a panel closed with its "x" button is reopened.
    for panel in PanelKind::ALL {
        let mut is_open = app.workspace.is_open(panel);
        if ui.checkbox(&mut is_open, panel.title()).changed() {
            app.workspace.set_open(panel, is_open);
        }
    }
    ui.separator();
    if ui.button("Reset Layout").clicked() {
        app.workspace.reset_layout();
    }
}

fn playback_menu(ui: &mut egui::Ui, app: &mut VizxApp) {
    let play_label = if app.is_playing() { "Pause" } else { "Play" };
    command_item(ui, app, play_label, "Space", AppCommand::TogglePlayback);
    command_item(ui, app, "Stop", "", AppCommand::StopPlayback);
    ui.separator();
    command_item(ui, app, "Go to Start", "Home", AppCommand::GoToStart);
    command_item(ui, app, "Go to End", "End", AppCommand::GoToEnd);
}

fn help_menu(ui: &mut egui::Ui, app: &mut VizxApp) {
    if ui.button("About VizX").clicked() {
        app.is_about_window_open = true;
    }
}

/// A greyed-out menu entry for a feature planned for a later milestone.
///
/// The entries are shown so the menu structure is already in place, but they
/// are disabled instead of pretending to work.
fn unavailable_item(ui: &mut egui::Ui, label: &str, shortcut: &str) {
    ui.add_enabled(false, egui::Button::new(label).shortcut_text(shortcut))
        .on_disabled_hover_text(NOT_AVAILABLE_YET);
}
