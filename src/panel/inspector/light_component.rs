use super::common::*;
use super::panel::InspectorPanel;
use super::resource_selector::ResourceSelector;
use crate::model::base::*;
use crate::model::scene::LightComponent;
use crate::model::scene::LightProperties;
use crate::model::scene::Properties;

use eframe::egui;
use uuid::Uuid;

impl InspectorPanel {
    pub fn show_light_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        component: &mut LightComponent,
        resource_selector: &ResourceSelector,
    ) -> bool {
        let mut is_changed = false;
        let t = component.get_type();
        let title = LightComponent::get_name_from_type(&t);
        let mut keys = Vec::new();
        let light_properties = LightProperties::get_instance();
        let light = component.get_light();
        let mut light = light.write().unwrap();
        let props = light.as_property_map_mut();
        if let Some(entries) = light_properties.get_entries(&t) {
            for entry in entries.iter() {
                let key_type = &entry.key_type;
                let key_name = &entry.key_name;
                let init = &entry.default_value;
                let range = &entry.value_range;
                if props.get(&key_name).is_none() {
                    let key = PropertyMap::get_key(key_type, key_name);
                    props.insert(&key, init.clone());
                }
                keys.push((key_type.clone(), key_name.clone(), range.clone()));
            }
        }
        //-------------------------------------------------------------------
        if show_component_props(index, &title, ui, props, &keys, resource_selector) {
            is_changed = true;
            props.add_string("string edition", &Uuid::new_v4().to_string());
        }
        return is_changed;
    }
}
