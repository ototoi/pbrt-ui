use super::super::panel::InspectorPanel;
//use super::super::common::*;
use crate::model::scene::Texture;


use eframe::egui;

impl InspectorPanel {
    pub fn show_texture_preview(&self, ui: &mut egui::Ui, width: f32, _texture: &Texture) {
        let width = width.min(ui.available_width());
        egui_extras::StripBuilder::new(ui)
            .size(egui_extras::Size::exact(width))
            .vertical(|mut strip| {
                strip.strip(|builder| {
                    builder
                        .size(egui_extras::Size::remainder())
                        .size(egui_extras::Size::exact(width))
                        .size(egui_extras::Size::remainder())
                        .horizontal(|mut strip| {
                            strip.empty();
                            strip.cell(|ui| {
                                let rect = ui.available_rect_before_wrap();
                                // Draw the texture preview
                                ui.painter().rect_filled(
                                    rect,
                                    0.0,
                                    egui::Color32::from_rgb(128, 128, 0),
                                );
                            });
                            strip.empty();
                        });
                });
            });
    }
}
