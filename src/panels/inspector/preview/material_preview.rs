use super::super::panel::InspectorPanel;
//use super::super::common::*;
use crate::models::base::*;

use eframe::egui;

impl InspectorPanel {
    pub fn show_material_preview(&self, ui: &mut egui::Ui, width: f32, props: &mut PropertyMap) {
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
                                ui.painter().rect_filled(
                                    ui.available_rect_before_wrap(),
                                    0.0,
                                    egui::Color32::DARK_GREEN,
                                );
                                ui.vertical_centered(|ui| {
                                    ui.label("Material Preview");
                                });
                            });
                            strip.empty();
                        });
                });
            });
    }
}