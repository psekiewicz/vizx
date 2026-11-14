//! User interface of VizX.
//!
//! Each submodule draws one region of the window. They all read and modify
//! the same application state, so they are views of one editor rather than
//! independent tools.

use crate::app::AppCommand;

pub mod about_window;
pub mod menu_bar;
pub mod panels;
pub mod preset_browser;
pub mod project_settings_window;
pub mod status_bar;
pub mod toolbar;
pub mod unsaved_changes_dialog;
pub mod workspace;

/// The entries of the "Add" menu. Returns the command of the entry that
/// was clicked.
///
/// Everything that can be added to a project is listed here and only here,
/// so every "Add" menu in the application (menu bar, toolbar, media
/// library, context menus) offers the same things in the same order. A new
/// kind of content becomes available everywhere by adding one line.
pub fn add_menu_items(ui: &mut eframe::egui::Ui) -> Option<AppCommand> {
    let mut chosen = None;
    let mut item = |ui: &mut eframe::egui::Ui, label: &str, shortcut: &str, command| {
        let button = eframe::egui::Button::new(label).shortcut_text(shortcut);
        if ui.add(button).clicked() {
            chosen = Some(command);
        }
    };

    item(ui, "Audio File...", "Ctrl+I", AppCommand::ImportAudio);
    item(
        ui,
        "MilkDrop Preset...",
        "Ctrl+M",
        AppCommand::BrowseMilkdropPresets,
    );
    item(
        ui,
        "MilkDrop Preset File...",
        "",
        AppCommand::AddMilkdropPreset,
    );
    ui.separator();
    item(ui, "Visual Layer", "", AppCommand::AddVisualLayer);
    item(ui, "Audio Track", "", AppCommand::AddAudioTrack);

    chosen
}

/// An "Add" entry that opens the [`add_menu_items`] as a submenu, for use
/// inside other menus. A chosen command is written to `requested_command`.
pub fn add_submenu(ui: &mut eframe::egui::Ui, requested_command: &mut Option<AppCommand>) {
    ui.menu_button("Add", |ui| {
        if let Some(command) = add_menu_items(ui) {
            *requested_command = Some(command);
        }
    });
}

/// Formats a length of time as `m:ss`, e.g. `3:35`.
pub fn format_duration(milliseconds: u64) -> String {
    let total_seconds = (milliseconds + 500) / 1000;
    format!("{}:{:02}", total_seconds / 60, total_seconds % 60)
}

/// Tooltip for controls whose feature belongs to a later milestone.
pub const NOT_AVAILABLE_YET: &str = "Not available yet in this prototype";
