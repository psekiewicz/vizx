//! The window for choosing a visualizer plugin.
//!
//! It lists the plugins that ship with VizX and offers to load one from a
//! folder, which is how a plugin of the user's own gets into a project.

use eframe::egui;

use crate::app::{AppCommand, VizxApp};
use crate::theme;

/// State of the visualizer browser window.
#[derive(Default)]
pub struct VisualizerBrowser {
    pub is_open: bool,
    /// Project reference of the selected plugin.
    selected: Option<String>,
}

pub fn show(ctx: &egui::Context, app: &mut VizxApp) {
    let mut is_open = app.visualizer_browser.is_open;
    // (reference to store in the project, name to report)
    let mut plugin_to_add: Option<(String, String)> = None;
    let mut wants_folder = false;

    egui::Window::new("Visualizers")
        .open(&mut is_open)
        .collapsible(false)
        .default_size([460.0, 360.0])
        .min_size([320.0, 200.0])
        .default_pos(ctx.content_rect().center() - egui::vec2(230.0, 240.0))
        .show(ctx, |ui| {
            let plugins = app.visualizer_catalog.bundled();
            let selected = &mut app.visualizer_browser.selected;

            // Leave room below the list for the separator and the buttons.
            let spacing = ui.spacing();
            let footer_height = spacing.interact_size.y + spacing.item_spacing.y * 3.0 + 6.0;
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .max_height((ui.available_height() - footer_height).max(40.0))
                .show(ui, |ui| {
                    if plugins.is_empty() {
                        ui.label(
                            egui::RichText::new(
                                "The visualizers that ship with VizX were not found on this \
                                 computer.",
                            )
                            .color(theme::TEXT_MUTED),
                        );
                    }
                    ui.with_layout(egui::Layout::top_down_justified(egui::Align::LEFT), |ui| {
                        for plugin in plugins {
                            let is_selected = selected.as_deref() == Some(&plugin.reference);
                            let row = ui
                                .add(egui::Button::selectable(
                                    is_selected,
                                    plugin.manifest.name.as_str(),
                                ))
                                .on_hover_text(plugin_summary(&plugin.manifest));
                            if !plugin.manifest.description.is_empty() {
                                ui.label(
                                    egui::RichText::new(&plugin.manifest.description)
                                        .small()
                                        .color(theme::TEXT_MUTED),
                                );
                            }
                            ui.add_space(2.0);

                            if row.clicked() {
                                *selected = Some(plugin.reference.clone());
                            }
                            if row.double_clicked() {
                                plugin_to_add =
                                    Some((plugin.reference.clone(), plugin.manifest.name.clone()));
                            }
                        }
                    });
                });

            ui.separator();
            ui.horizontal(|ui| {
                if ui
                    .button("Load from Folder...")
                    .on_hover_text(
                        "Add a visualizer of your own: a folder with visualizer.json and \
                         shader.wgsl",
                    )
                    .clicked()
                {
                    wants_folder = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let chosen = plugins
                        .iter()
                        .find(|plugin| selected.as_deref() == Some(&plugin.reference));
                    let add_button = ui
                        .add_enabled(chosen.is_some(), egui::Button::new("Add to Project"))
                        .on_hover_text("Enter or double-click");
                    if add_button.clicked()
                        && let Some(plugin) = chosen
                    {
                        plugin_to_add =
                            Some((plugin.reference.clone(), plugin.manifest.name.clone()));
                    }
                });
            });
        });

    app.visualizer_browser.is_open = is_open;
    if let Some((reference, name)) = plugin_to_add {
        app.session.add_visualizer_plugin(&reference);
        app.status_message = format!("Added visualizer {name}");
    }
    if wants_folder {
        // Opening a file dialog is an application command, like every other
        // action that needs the main window.
        app.request(AppCommand::AddVisualizerFolder);
    }
}

fn plugin_summary(manifest: &vizx_core::Manifest) -> String {
    if manifest.author.is_empty() {
        manifest.name.clone()
    } else {
        format!("{} by {}", manifest.name, manifest.author)
    }
}
