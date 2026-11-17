//! User interface of VizX.
//!
//! Each submodule draws one region of the window. They all read and modify
//! the same application state, so they are views of one editor rather than
//! independent tools.

use crate::app::AppCommand;

pub mod about_window;
pub mod export_window;
pub mod lyrics_window;
pub mod menu_bar;
pub mod panels;
pub mod preset_browser;
pub mod project_settings_window;
pub mod recovery_window;
pub mod shortcuts_window;
pub mod status_bar;
pub mod subtitle_editor;
pub mod toolbar;
pub mod unsaved_changes_dialog;
pub mod visualizer_browser;
pub mod welcome_window;
pub mod workspace;

/// The entries of the "Add" menu. Returns the command of the entry that
/// was clicked.
///
/// Everything that can be added to a project is listed here and only here.
/// The menu is offered in one place: the context menu of the timeline
/// (right-click on a track, a lane or the empty area). The entries with a
/// shortcut can also be reached from the keyboard.
pub fn add_menu_items(ui: &mut eframe::egui::Ui) -> Option<AppCommand> {
    let mut chosen = None;
    let mut item = |ui: &mut eframe::egui::Ui, label: &str, shortcut: &str, command| {
        let button = eframe::egui::Button::new(label).shortcut_text(shortcut);
        if ui.add(button).clicked() {
            chosen = Some(command);
        }
    };

    item(ui, "Audio File...", "Ctrl+I", AppCommand::ImportAudio);
    item(ui, "Image File...", "Ctrl+Shift+I", AppCommand::ImportImage);
    item(
        ui,
        "Background Image...",
        "",
        AppCommand::ImportBackgroundImage,
    );
    item(ui, "Video or GIF...", "", AppCommand::ImportVideo);
    item(ui, "Image Sequence...", "", AppCommand::ImportImageSequence);
    item(ui, "Sprite Sheet...", "", AppCommand::ImportSpriteSheet);
    item(
        ui,
        "Background Video...",
        "",
        AppCommand::ImportBackgroundVideo,
    );
    item(ui, "Text", "Ctrl+T", AppCommand::AddText);
    item(ui, "Effects Layer", "", AppCommand::AddEffectsLayer);
    item(
        ui,
        "Lyrics / Subtitles...",
        "Ctrl+L",
        AppCommand::OpenLyrics,
    );
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
    item(
        ui,
        "Visualizer...",
        "Ctrl+Shift+M",
        AppCommand::BrowseVisualizers,
    );
    ui.separator();
    item(ui, "Visual Layer", "", AppCommand::AddVisualLayer);
    item(ui, "Subtitle Layer", "", AppCommand::AddSubtitleLayer);
    item(ui, "Audio Track", "", AppCommand::AddAudioTrack);

    chosen
}

/// An "Add" entry that opens the [`add_menu_items`] as a submenu of the
/// timeline's context menus. A chosen command is written to `requested_command`.
pub fn add_submenu(ui: &mut eframe::egui::Ui, requested_command: &mut Option<AppCommand>) {
    ui.menu_button("Add", |ui| {
        if let Some(command) = add_menu_items(ui) {
            *requested_command = Some(command);
        }
    });
}

/// The [`add_menu_items`] listed directly under an "Add" heading, for a
/// context menu that offers nothing else. This saves the extra step of
/// opening a submenu that would be the menu's only entry.
pub fn add_menu_section(ui: &mut eframe::egui::Ui, requested_command: &mut Option<AppCommand>) {
    ui.label(eframe::egui::RichText::new("Add").strong());
    ui.separator();
    if let Some(command) = add_menu_items(ui) {
        *requested_command = Some(command);
    }
}

/// Formats a length of time as `m:ss`, e.g. `3:35`.
pub fn format_duration(milliseconds: u64) -> String {
    let total_seconds = (milliseconds + 500) / 1000;
    format!("{}:{:02}", total_seconds / 60, total_seconds % 60)
}
