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
        "Playback and moving around",
        &[
            ("Space", "Play / pause"),
            ("Home / End", "Go to the start / end"),
            ("Left / Right", "One frame back / forward"),
            ("Shift+Left / Right", "One second back / forward"),
            ("Up / Down", "Previous / next clip start or end, or marker"),
            ("Ctrl+Left / Right", "Previous / next marker"),
            ("M", "Add a marker at the playhead"),
            ("L", "Loop on / off"),
            ("I / O", "Loop starts / ends at the playhead"),
            ("Shift+drag on the ruler", "Mark the stretch to loop"),
        ],
    ),
    (
        "Editing",
        &[
            ("Ctrl+Z", "Undo"),
            ("Ctrl+Shift+Z or Ctrl+Y", "Redo"),
            ("Ctrl+C / Ctrl+X", "Copy / cut the selected clip"),
            ("Ctrl+V", "Paste the clip at the playhead"),
            ("Ctrl+K", "Split the selected clip at the playhead"),
            ("Ctrl+D", "Duplicate the selected clip"),
            ("Delete", "Delete the selected clip or track"),
            ("B", "Place a beat at the playhead (selected song)"),
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
        "Lyrics window, while tapping",
        &[
            ("Enter", "Next line starts now"),
            ("Backspace", "End the line now (a pause)"),
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
                egui::RichText::new("Single keys work while no text field is being typed into.")
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
