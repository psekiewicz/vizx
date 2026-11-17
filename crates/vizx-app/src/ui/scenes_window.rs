//! The Scenes window: the song's parts, each starting at a marker, and the
//! transition into each of them (cut, flash, fade through black, zoom
//! punch, glitch, blur, spin, shake).

use eframe::egui;
use vizx_project::{Transition, TransitionKind};

use crate::app::VizxApp;
use crate::theme;
use crate::ui::format_duration;

#[derive(Default)]
pub struct ScenesWindow {
    pub is_open: bool,
}

pub fn show(ctx: &egui::Context, app: &mut VizxApp) {
    if !app.scenes.is_open {
        return;
    }
    let mut is_open = true;
    egui::Window::new("Scenes")
        .open(&mut is_open)
        .collapsible(false)
        .resizable(true)
        .default_size([520.0, 360.0])
        .show(ctx, |ui| contents(ui, app));
    app.scenes.is_open = is_open;
}

fn contents(ui: &mut egui::Ui, app: &mut VizxApp) {
    ui.label(
        egui::RichText::new(
            "Every marker starts a scene. Choose how the picture changes where a scene \
             starts; the transitions are drawn on the \"Scene transitions\" layer, above the \
             picture and below the subtitles.",
        )
        .small()
        .color(theme::TEXT_MUTED),
    );
    ui.horizontal(|ui| {
        if ui
            .button("New Scene at Playhead")
            .on_hover_text("Adds a marker at the playhead (also: M)")
            .clicked()
        {
            app.session.add_marker_at_playhead();
        }
        if ui
            .button("Same Transition Everywhere")
            .on_hover_text("Give every scene the transition of the first one that has one")
            .clicked()
        {
            let first = app
                .session
                .project()
                .markers()
                .iter()
                .map(|marker| marker.transition)
                .find(|transition| transition.kind != TransitionKind::Cut);
            if let Some(transition) = first {
                let count = app.session.project().markers().len();
                app.session.edit_project(|project| {
                    // Every scene, so no short cut that stops at the first.
                    let mut changed = false;
                    for index in 0..count {
                        changed |= project.set_marker_transition(index, transition);
                    }
                    changed
                });
            }
        }
    });
    ui.separator();

    let scenes = app.session.project().scenes();
    let playhead = (app.session.playhead_seconds() * 1000.0) as u64;
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .show(ui, |ui| {
            egui::Grid::new("scenes")
                .num_columns(5)
                .striped(true)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Scene").strong());
                    ui.label(egui::RichText::new("Time").strong());
                    ui.label(egui::RichText::new("Transition in").strong());
                    ui.label(egui::RichText::new("Length").strong());
                    ui.label("");
                    ui.end_row();
                    for scene in &scenes {
                        let is_now = scene.start_milliseconds <= playhead
                            && playhead < scene.end_milliseconds;
                        let Some(index) = scene.marker else {
                            ui.label(if is_now { "▶ Start" } else { "  Start" });
                            ui.label(format_duration(scene.start_milliseconds));
                            ui.label(egui::RichText::new("(beginning)").color(theme::TEXT_MUTED));
                            ui.label("");
                            ui.label("");
                            ui.end_row();
                            continue;
                        };
                        ui.horizontal(|ui| {
                            ui.label(if is_now { "▶" } else { "  " });
                            let mut name = scene.name.clone();
                            if ui
                                .add(egui::TextEdit::singleline(&mut name).desired_width(140.0))
                                .changed()
                            {
                                app.session
                                    .edit_project(|project| project.rename_marker(index, &name));
                            }
                        });
                        if ui
                            .link(format_duration(scene.start_milliseconds))
                            .on_hover_text("Go there")
                            .clicked()
                        {
                            app.session
                                .set_playhead_seconds(scene.start_milliseconds as f64 / 1000.0);
                        }
                        let mut transition = scene.transition;
                        egui::ComboBox::from_id_salt(("scene_transition", index))
                            .selected_text(transition.kind.label())
                            .width(150.0)
                            .show_ui(ui, |ui| {
                                for kind in TransitionKind::ALL {
                                    ui.selectable_value(&mut transition.kind, kind, kind.label());
                                }
                            });
                        let (shortest, longest) = Transition::DURATION_RANGE;
                        let mut seconds = transition.duration_milliseconds as f64 / 1000.0;
                        ui.add_enabled(
                            transition.kind != TransitionKind::Cut,
                            egui::DragValue::new(&mut seconds)
                                .range(shortest as f64 / 1000.0..=longest as f64 / 1000.0)
                                .speed(0.02)
                                .max_decimals(2)
                                .suffix(" s"),
                        );
                        transition.duration_milliseconds = (seconds * 1000.0).round() as u64;
                        if transition != scene.transition {
                            app.session.edit_project(|project| {
                                project.set_marker_transition(index, transition)
                            });
                        }
                        if ui
                            .small_button("Delete")
                            .on_hover_text("Remove the marker; the scene joins the one before")
                            .clicked()
                        {
                            app.session
                                .edit_project(|project| project.remove_marker(index));
                        }
                        ui.end_row();
                    }
                });
        });
}
