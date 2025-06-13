use super::common::*;
use super::panel::InspectorPanel;
use crate::models::base::*;
use crate::models::scene::MaterialComponent;

use eframe::egui;

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
        let mut name = props
            .find_one_string("string name_")
            .unwrap_or("".to_string());
        egui::TopBottomPanel::top(format!("{}_{}", title, index))
            .min_height(MIN_COMPONENT_HEIGHT)
            .show_inside(ui, |ui| {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label("Material");
                    ui.separator();
                    ui.text_edit_singleline(&mut name);
                });
                ui.separator();
                show_type(ui, props, &material_types);
                ui.separator();
                self.show_material_preview(ui, 300.0, props);
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
}
