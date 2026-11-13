use eframe::egui;

use crate::theme;

/// Media categories shown in the library. The project model has no media
/// sources yet, so every category is empty in this milestone.
const CATEGORIES: [&str; 4] = ["Audio Sources", "Images", "Animations", "Effects"];

pub fn show(ui: &mut egui::Ui) {
    for category in CATEGORIES {
        egui::CollapsingHeader::new(category)
            .default_open(true)
            .show(ui, |ui| {
                ui.label(
                    egui::RichText::new("No items")
                        .small()
                        .color(theme::TEXT_MUTED),
                );
            });
    }
}
