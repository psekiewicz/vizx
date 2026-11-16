//! The window for choosing one of the MilkDrop presets that ship with VizX.
//!
//! It is an egui window inside the main window, like Project Settings. The
//! collection is large (thousands of files), so it is indexed once on a
//! background thread and only the rows that are visible are drawn.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::thread;

use eframe::egui;
use vizx_core::{PresetEntry, PresetLibrary};

use crate::app::VizxApp;
use crate::theme;

const ALL_CATEGORIES: &str = "All categories";

/// Where the index of the bundled presets currently stands.
enum LibraryState {
    /// The browser has not been opened yet, so nothing was read from disk.
    NotLoaded,
    /// A background thread is reading the folder; its result arrives here.
    Loading(Receiver<Result<PresetLibrary, String>>),
    Ready(PresetLibrary),
    /// The presets cannot be offered; the text says why.
    Unavailable(String),
}

/// State of the preset browser window.
pub struct PresetBrowser {
    pub is_open: bool,
    /// Folder of the bundled presets, if this installation has one.
    bundled_presets: Option<PathBuf>,
    library: LibraryState,
    /// Text typed into the search field.
    query: String,
    /// The chosen category, or `None` for all of them.
    category: Option<String>,
    selected: Option<PresetEntry>,
    /// Set when the window opens, so the search field can be typed into
    /// straight away.
    should_focus_search: bool,
}

impl PresetBrowser {
    pub fn new(bundled_presets: Option<PathBuf>) -> Self {
        Self {
            is_open: false,
            bundled_presets,
            library: LibraryState::NotLoaded,
            query: String::new(),
            category: None,
            selected: None,
            should_focus_search: false,
        }
    }

    /// Opens the window. The first time, this also starts indexing the
    /// presets in the background.
    pub fn open(&mut self, ctx: &egui::Context) {
        self.is_open = true;
        self.should_focus_search = true;

        if matches!(self.library, LibraryState::NotLoaded) {
            let (sender, receiver) = mpsc::channel();
            let bundled_presets = self.bundled_presets.clone();
            let ctx = ctx.clone();
            thread::spawn(move || {
                // The window may have been closed meanwhile; then nobody
                // receives the result and that is fine.
                let _ = sender.send(load_library(bundled_presets));
                ctx.request_repaint();
            });
            self.library = LibraryState::Loading(receiver);
        }
    }

    /// Takes over the index once the background thread has finished it.
    fn collect_loaded(&mut self) {
        if let LibraryState::Loading(receiver) = &self.library
            && let Ok(result) = receiver.try_recv()
        {
            self.library = match result {
                Ok(library) => LibraryState::Ready(library),
                Err(reason) => LibraryState::Unavailable(reason),
            };
        }
    }
}

/// Runs on the background thread.
fn load_library(bundled_presets: Option<PathBuf>) -> Result<PresetLibrary, String> {
    let folder = bundled_presets.ok_or("Bundled presets not found.")?;
    let library = PresetLibrary::scan(&folder)
        .map_err(|error| format!("Could not read {}: {error}", folder.display()))?;

    if library.entries().is_empty() {
        // Typical when VizX is built from source and the preset collection
        // (a git submodule) was not downloaded.
        return Err(format!(
            "No presets in {}.\nWhen building from source, run: git submodule update --init",
            folder.display()
        ));
    }
    Ok(library)
}

pub fn show(ctx: &egui::Context, app: &mut VizxApp) {
    app.preset_browser.collect_loaded();

    let mut is_open = app.preset_browser.is_open;
    let mut preset_to_add = None;

    egui::Window::new("MilkDrop Presets")
        .open(&mut is_open)
        .collapsible(false)
        .default_size([760.0, 480.0])
        .min_size([380.0, 240.0])
        .default_pos(ctx.content_rect().center() - egui::vec2(380.0, 300.0))
        .show(ctx, |ui| {
            let browser = &mut app.preset_browser;
            match &browser.library {
                LibraryState::NotLoaded | LibraryState::Loading(_) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Reading presets...");
                    });
                }
                LibraryState::Unavailable(reason) => {
                    ui.label(egui::RichText::new(reason).color(theme::TEXT_MUTED));
                    ui.add_space(4.0);
                    ui.label("To load a .milk file, use Add > MilkDrop Preset File...");
                }
                LibraryState::Ready(library) => {
                    preset_to_add = browse(
                        ui,
                        library,
                        &mut browser.query,
                        &mut browser.category,
                        &mut browser.selected,
                        &mut browser.should_focus_search,
                    );
                }
            }
        });

    app.preset_browser.is_open = is_open;
    if let Some(entry) = preset_to_add {
        app.session.add_bundled_milkdrop_preset(&entry);
        app.status_message = format!("Added preset {}", entry.name);
    }
}

/// Draws the search field, the category list and the results. Returns the
/// preset the user asked to add to the project, if any.
fn browse(
    ui: &mut egui::Ui,
    library: &PresetLibrary,
    query: &mut String,
    category: &mut Option<String>,
    selected: &mut Option<PresetEntry>,
    should_focus_search: &mut bool,
) -> Option<PresetEntry> {
    const CATEGORY_LIST_WIDTH: f32 = 150.0;
    let mut preset_to_add = None;

    // The search field keeps the keyboard focus, so arrows and Enter are
    // taken before it sees them: Up / Down walk the list, Enter adds.
    let search_id = ui.id().with("preset_search");
    let mut step = 0_i64;
    let mut wants_add = false;
    if ui.memory(|memory| memory.has_focus(search_id)) {
        ui.input_mut(|input| {
            use egui::{Key, Modifiers};
            if input.consume_key(Modifiers::NONE, Key::ArrowDown) {
                step = 1;
            }
            if input.consume_key(Modifiers::NONE, Key::ArrowUp) {
                step = -1;
            }
            if input.consume_key(Modifiers::NONE, Key::PageDown) {
                step = 10;
            }
            if input.consume_key(Modifiers::NONE, Key::PageUp) {
                step = -10;
            }
            wants_add = input.consume_key(Modifiers::NONE, Key::Enter);
        });
    }

    ui.horizontal(|ui| {
        let search_width =
            ui.available_width() - CATEGORY_LIST_WIDTH - ui.spacing().item_spacing.x * 3.0;
        let search_field = ui.add(
            egui::TextEdit::singleline(query)
                .id(search_id)
                .hint_text("Search")
                .desired_width(search_width.max(80.0)),
        );
        if *should_focus_search {
            search_field.request_focus();
            *should_focus_search = false;
        }

        egui::ComboBox::from_id_salt("preset_category")
            .width(CATEGORY_LIST_WIDTH)
            .selected_text(category.as_deref().map_or(ALL_CATEGORIES, category_label))
            .show_ui(ui, |ui| {
                ui.selectable_value(category, None, ALL_CATEGORIES);
                for name in library.categories() {
                    ui.selectable_value(category, Some(name.clone()), category_label(name));
                }
            });
    });
    ui.separator();

    let results = library.search(category.as_deref(), query);

    // Leave room below the list for the separator and the button row.
    let spacing = ui.spacing();
    let row_height = spacing.interact_size.y;
    let row_pitch = row_height + spacing.item_spacing.y;
    let footer_height = row_height + spacing.item_spacing.y * 3.0 + 6.0;
    let list_height = (ui.available_height() - footer_height).max(row_height);

    let mut scroll_area = egui::ScrollArea::vertical()
        .auto_shrink(false)
        .max_height(list_height);
    if step != 0 && !results.is_empty() {
        let current = selected
            .as_ref()
            .and_then(|entry| results.iter().position(|result| *result == entry));
        let index = match current {
            Some(index) => (index as i64 + step).clamp(0, results.len() as i64 - 1) as usize,
            None => 0,
        };
        *selected = Some(PresetEntry::clone(results[index]));
        // Scroll just enough to show the row.
        let offset_id = ui.id().with("preset_list_offset");
        let offset: f32 = ui.data(|data| data.get_temp(offset_id)).unwrap_or(0.0);
        let top = index as f32 * row_pitch;
        let new_offset = if top < offset {
            top
        } else if top + row_pitch > offset + list_height {
            top + row_pitch - list_height
        } else {
            offset
        };
        scroll_area = scroll_area.vertical_scroll_offset(new_offset);
    }
    if wants_add {
        preset_to_add = selected.clone();
    }
    let output = scroll_area
        // Only the rows inside the visible range are laid out, which keeps
        // a list of thousands of presets as cheap as a short one.
        .show_rows(ui, row_height, results.len(), |ui, visible_rows| {
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                for &entry in &results[visible_rows] {
                    let is_selected = selected.as_ref() == Some(entry);
                    let folder = egui::RichText::new(folder_label(entry, category.is_none()))
                        .small()
                        .color(theme::TEXT_MUTED);
                    let row = ui.add(
                        egui::Button::selectable(is_selected, entry.name.as_str())
                            .right_text(folder)
                            .truncate(),
                    );
                    // Long names are cut off in the list; show them in full.
                    let row = row.on_hover_text(entry.name.as_str());
                    if row.clicked() {
                        *selected = Some(entry.clone());
                        // Back to the search field, so the keys keep working.
                        ui.memory_mut(|memory| memory.request_focus(search_id));
                    }
                    if row.double_clicked() {
                        preset_to_add = Some(entry.clone());
                    }
                }
            });
        });
    let offset_id = ui.id().with("preset_list_offset");
    ui.data_mut(|data| data.insert_temp(offset_id, output.state.offset.y));

    ui.separator();
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{} of {} presets",
                results.len(),
                library.entries().len()
            ))
            .small()
            .color(theme::TEXT_MUTED),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let add_button = ui
                .add_enabled(selected.is_some(), egui::Button::new("Add to Project"))
                .on_hover_text("Enter or double-click");
            if add_button.clicked() {
                preset_to_add = selected.clone();
            }
        });
    });

    preset_to_add
}

/// Name shown for a category. Presets lying loose in a pack have none.
fn category_label(category: &str) -> &str {
    if category.is_empty() {
        "Uncategorized"
    } else {
        category
    }
}

/// Where a preset lies, shown dimmed at the right of its row. The category
/// is left out when the list is already limited to one category.
fn folder_label(entry: &PresetEntry, with_category: bool) -> String {
    match (with_category, entry.subfolder.is_empty()) {
        (true, true) => category_label(&entry.category).to_owned(),
        (true, false) => format!("{} / {}", category_label(&entry.category), entry.subfolder),
        (false, _) => entry.subfolder.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_bundled_presets_are_reported_instead_of_an_empty_list() {
        assert!(load_library(None).is_err());

        let empty_folder =
            std::env::temp_dir().join(format!("vizx-preset-browser-empty-{}", std::process::id()));
        std::fs::create_dir_all(empty_folder.join("pack")).unwrap();
        let result = load_library(Some(empty_folder.clone()));
        let _ = std::fs::remove_dir_all(&empty_folder);

        assert!(result.unwrap_err().contains("git submodule update --init"));
    }
}
