use super::common::*;
use super::panel::InspectorPanel;
use crate::models::base::*;
use crate::models::scene::Properties;

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
    pub fn show_typed_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        title: &str,
        props: &mut PropertyMap,
        properties: &impl Properties,
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
                show_properties(index, ui, props, &keys);

                ui.add_space(3.0);
            });
    }
}
