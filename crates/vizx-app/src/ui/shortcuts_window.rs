//! Help > Keyboard Shortcuts (F1): every shortcut of VizX in one list, so
//! they can be learned without hunting through menus.

use eframe::egui;

use crate::app::VizxApp;
use crate::theme;

/// The shortcuts, in groups. Keep in step with
/// `VizxApp::handle_keyboard_shortcuts` and the timeline and preview panels.
const GROUPS: [(&str, &[(&str, &str)]); 5] = [
    (
        "Project",
        &[
            ("Ctrl+N", "New project"),
            ("Ctrl+O", "Open project"),
            ("Ctrl+S", "Save"),
            ("Ctrl+Shift+S", "Save as"),
            ("Ctrl+E", "Export video"),
            ("F1", "This list"),
        ],
    ),
    (
        "Playback & navigation",
        &[
            ("Space", "Play / pause"),
            ("Home / End", "Go to start / end"),
            ("Left / Right", "One frame back / forward"),
            ("Shift+Left / Right", "One second back / forward"),
            ("Up / Down", "Previous / next edit point"),
            ("Ctrl+Left / Right", "Previous / next marker"),
            ("M", "Add marker"),
            ("L", "Loop on / off"),
            ("I / O", "Set loop in / out"),
            ("Shift+drag on the ruler", "Set loop range"),
        ],
    ),
    (
        "Editing",
        &[
            ("Ctrl+Z", "Undo"),
            ("Ctrl+Shift+Z or Ctrl+Y", "Redo"),
            ("Ctrl+click a clip", "Add to / remove from selection"),
            ("Drag on an empty area", "Box select"),
            ("Ctrl+A", "Select all clips"),
            ("Ctrl+C / Ctrl+X", "Copy / cut"),
            ("Ctrl+V", "Paste at playhead"),
            ("Ctrl+K", "Split at playhead"),
            ("Ctrl+D", "Duplicate"),
            ("Delete", "Delete"),
            ("B", "Tap a beat (selected audio clip)"),
        ],
    ),
    (
        "Adding",
        &[
            ("Ctrl+I", "Audio file"),
            ("Ctrl+Shift+I", "Image file"),
            ("Ctrl+T", "Text"),
            ("Ctrl+L", "Lyrics / subtitles"),
            ("Ctrl+M", "MilkDrop preset"),
            ("Ctrl+Shift+M", "Visualizer"),
            ("Right-click the timeline", "Everything else"),
        ],
    ),
    (
        "Lyrics tapping",
        &[
            ("Enter", "Next line"),
            ("Backspace", "End line"),
            ("Esc", "Stop tapping"),
        ],
    ),
];

pub fn show(ctx: &egui::Context, app: &mut VizxApp) {
    egui::Window::new("Keyboard Shortcuts")
        .open(&mut app.is_shortcuts_open)
        .collapsible(false)
        .resizable(false)
        .default_pos(ctx.content_rect().center() - egui::vec2(220.0, 260.0))
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new("Single-key shortcuts are off while typing in a text field.")
                    .small()
                    .color(theme::TEXT_MUTED),
            );
            for (title, shortcuts) in GROUPS {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(title).strong());
                ui.separator();
                egui::Grid::new(title)
                    .num_columns(2)
                    .spacing([24.0, 3.0])
                    .show(ui, |ui| {
                        for (keys, action) in shortcuts {
                            ui.label(egui::RichText::new(*keys).monospace());
                            ui.label(*action);
                            ui.end_row();
                        }
                    });
            }
        });
}
