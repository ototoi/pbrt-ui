use super::common::*;
use super::panel::InspectorPanel;
use super::resource_selector::ResourceSelector;
use crate::model::base::*;
use crate::model::scene::Properties;

use eframe::egui;

impl InspectorPanel {
    pub fn show_typed_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        title: &str,
        props: &mut PropertyMap,
        properties: &impl Properties,
        resource_selector: &ResourceSelector,
    ) {
        egui::TopBottomPanel::top(format!("{}_{}", title, index))
            .min_height(MIN_COMPONENT_HEIGHT)
            .show_inside(ui, |ui| {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(title);
                });
                ui.separator();
                let types = properties.get_types();
                show_type(ui, props, &types);
                ui.separator();
                let t = props.find_one_string("string type").unwrap();
                let mut keys = Vec::new();
                let entries = properties.get_entries(&t);
                for (key_type, key_name, init, range) in entries.iter() {
                    if props.get(key_name).is_none() {
                        let key = PropertyMap::get_key(key_type, key_name);
                        props.insert(&key, init.clone());
                    }
                    keys.push((key_type.clone(), key_name.clone(), range.clone()));
                }
                show_properties(index, ui, props, &keys, resource_selector);

                ui.add_space(3.0);
            });
    }
}
