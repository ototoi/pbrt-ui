use super::common::*;
use super::panel::InspectorPanel;
use crate::models::base::*;
use crate::models::scene::MaterialComponent;

use eframe::egui;

pub fn show_type(ui: &mut egui::Ui, props: &mut PropertyMap, types: &[String]) {
    if let Some(v) = props.get_mut("type") {
        if let Property::Strings(s) = v {
            egui::ComboBox::from_id_salt("type")
                .selected_text(s[0].clone())
                .show_ui(ui, |ui| {
                    for name in types.iter() {
                        ui.selectable_value(&mut s[0], name.clone(), name.clone());
                    }
                });
        }
    }
}

impl InspectorPanel {
    pub fn show_material_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        component: &mut MaterialComponent,
    ) {
        let material = component.material.clone();
        let mut material = material.write().unwrap();
        let props = material.as_property_map_mut();
        self.show_material_props(index, "Material", ui, props);
    }

    fn show_material_props(
        &self,
        index: usize,
        title: &str,
        ui: &mut egui::Ui,
        props: &mut PropertyMap,
    ) {
        let material_types = self.material_parameters.get_types();
        let mut name = "";
        let names = props.get_strings("string name_");
        if !names.is_empty() {
            name = &names[0];
        }
        let title = format!("{}: {}", title, name);
        egui::TopBottomPanel::top(format!("{}_{}", title, index))
            .min_height(MIN_COMPONENT_HEIGHT)
            .show_inside(ui, |ui| {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(title);
                });
                ui.separator();

                show_type(ui, props, &material_types);
                ui.separator();
                self.show_material_preview(ui, 150.0, props);
                ui.separator();
                let mut keys = Vec::new();
                let mat_type = props.find_one_string("string type").unwrap();
                if let Some(params) = self.material_parameters.get(&mat_type) {
                    for (key_type, key_name, init, range) in params.iter() {
                        if props.get(key_name).is_none() {
                            let key = PropertyMap::get_key(key_type, key_name);
                            props.insert(&key, init.clone());
                        }
                        keys.push((key_type.clone(), key_name.clone(), range.clone()));
                    }
                    show_properties(index, ui, props, &keys);
                } else {
                    ui.horizontal(|ui| {
                        ui.label("No parameters found for this material type");
                    });
                }
                ui.add_space(3.0);
            });
    }

    fn show_material_preview(&self, ui: &mut egui::Ui, width: f32, props: &mut PropertyMap) {
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
                                    egui::Color32::WHITE,
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
