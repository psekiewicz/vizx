use eframe::egui;

use crate::app::{AppCommand, VizxApp};
use crate::ui::workspace::PanelKind;

pub fn show(ui: &mut egui::Ui, app: &mut VizxApp) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("File", |ui| file_menu(ui, app));
        ui.menu_button("Edit", |ui| edit_menu(ui, app));
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
    let recent = app.settings.recent_projects.clone();
    ui.add_enabled_ui(!recent.is_empty(), |ui| {
        ui.menu_button("Open Recent", |ui| {
            for (index, path) in recent.iter().enumerate() {
                let name = path.file_stem().map_or_else(
                    || path.display().to_string(),
                    |stem| stem.to_string_lossy().into_owned(),
                );
                let button = ui
                    .add_enabled(path.is_file(), egui::Button::new(name))
                    .on_hover_text(path.display().to_string())
                    .on_disabled_hover_text("This file no longer exists");
                if button.clicked() {
                    app.request(AppCommand::OpenRecent(index));
                }
            }
        });
    });
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
        "Project Settings...",
        "",
        AppCommand::OpenProjectSettings,
    );
    command_item(
        ui,
        app,
        "Export Video...",
        "Ctrl+E",
        AppCommand::ExportVideo,
    );
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

fn edit_menu(ui: &mut egui::Ui, app: &mut VizxApp) {
    history_item(
        ui,
        app,
        "Undo",
        "Ctrl+Z",
        AppCommand::Undo,
        app.session.can_undo(),
    );
    history_item(
        ui,
        app,
        "Redo",
        "Ctrl+Shift+Z",
        AppCommand::Redo,
        app.session.can_redo(),
    );
    ui.separator();
    command_item(ui, app, "Cut Clip", "Ctrl+X", AppCommand::CutClip);
    command_item(ui, app, "Copy Clip", "Ctrl+C", AppCommand::CopyClip);
    command_item(ui, app, "Paste Clip", "Ctrl+V", AppCommand::PasteClip);
    command_item(ui, app, "Delete", "Del", AppCommand::DeleteSelection);
    ui.separator();
    command_item(
        ui,
        app,
        "Split Clip at Playhead",
        "Ctrl+K",
        AppCommand::SplitClip,
    );
    command_item(
        ui,
        app,
        "Duplicate Clip",
        "Ctrl+D",
        AppCommand::DuplicateClip,
    );
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
    ui.separator();
    command_item(
        ui,
        app,
        "Previous Frame",
        "Left",
        AppCommand::MovePlayheadFrames(-1),
    );
    command_item(
        ui,
        app,
        "Next Frame",
        "Right",
        AppCommand::MovePlayheadFrames(1),
    );
    command_item(
        ui,
        app,
        "Previous Clip Edge",
        "Up",
        AppCommand::PreviousClipEdge,
    );
    command_item(ui, app, "Next Clip Edge", "Down", AppCommand::NextClipEdge);
}

fn help_menu(ui: &mut egui::Ui, app: &mut VizxApp) {
    command_item(
        ui,
        app,
        "Keyboard Shortcuts",
        "F1",
        AppCommand::ShowShortcuts,
    );
    if ui.button("About VizX").clicked() {
        app.is_about_window_open = true;
    }
}

/// Undo or Redo: greyed out when there is nothing to undo or redo.
fn history_item(
    ui: &mut egui::Ui,
    app: &mut VizxApp,
    label: &str,
    shortcut: &str,
    command: AppCommand,
    is_available: bool,
) {
    let button = egui::Button::new(label).shortcut_text(shortcut);
    if ui.add_enabled(is_available, button).clicked() {
        app.request(command);
    }
}
