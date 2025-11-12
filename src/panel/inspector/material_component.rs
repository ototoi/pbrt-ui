use super::common::*;
use super::panel::InspectorPanel;
use super::resource_selector::ResourceSelector;
use crate::model::base::*;
use crate::model::scene::MaterialComponent;
use crate::model::scene::MaterialProperties;
use crate::model::scene::Properties;

use eframe::egui;
use uuid::Uuid;

impl InspectorPanel {
    pub fn show_material_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        component: &mut MaterialComponent,
        resource_selector: &ResourceSelector,
    ) -> bool {
        let material = component.get_material();
        let mut material = material.write().unwrap();
        let props = material.as_property_map_mut();
        return self.show_material_props(index, "Material", ui, props, resource_selector);
    }

    fn show_material_props(
        &self,
        index: usize,
        title: &str,
        ui: &mut egui::Ui,
        props: &mut PropertyMap,
        resource_selector: &ResourceSelector,
    ) -> bool {
        let mut is_changed = false;
        let material_properties = MaterialProperties::get_instance();
        let material_types = material_properties.get_types();

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
                if show_type(ui, props, &material_types) {
                    is_changed = true;
                }
                ui.separator();
                self.show_material_preview(ui, 300.0, props);
                ui.separator();
                let mut keys = Vec::new();
                let mat_type = props.find_one_string("string type").unwrap();
                let mut hide_sigma = false;
                if mat_type == "subsurface" {
                    let name_value = props
                        .find_one_string("string name")
                        .unwrap_or("".to_string());
                    if !name_value.is_empty() {
                        hide_sigma = true;
                    }
                }
                if let Some(params) = material_properties.get_entries(&mat_type) {
                    for entry in params.iter() {
                        if hide_sigma {
                            if entry.key_name == "sigma_a" || entry.key_name == "sigma_s" {
                                continue;
                            }
                        }
                        if props.get(&entry.key_name).is_none() {
                            let key = PropertyMap::get_key(&entry.key_type, &entry.key_name);
                            props.insert(&key, entry.default_value.clone());
                        }
                        keys.push((
                            entry.key_type.clone(),
                            entry.key_name.clone(),
                            entry.value_range.clone(),
                        ));
                    }
                    if show_properties(index, ui, props, &keys, resource_selector) {
                        is_changed = true;
                        props.add_string("string edition", &Uuid::new_v4().to_string());
                    }
                } else {
                    ui.horizontal(|ui| {
                        ui.label("No parameters found for this material type");
                    });
                }
                ui.add_space(3.0);
            });
        return is_changed;
    }
}
