use super::common::*;
use super::panel::InspectorPanel;
use super::resource_selector::ResourceSelector;
use crate::model::base::*;
use crate::model::scene::LightComponent;
use crate::model::scene::Properties;

use eframe::egui;

impl InspectorPanel {
    pub fn show_light_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        component: &mut LightComponent,
        resource_selector: &ResourceSelector,
    ) {
        let t = component.get_type();
        let title = LightComponent::get_name_from_type(&t);
        let mut keys = Vec::new();
        let entries = self.light_properties.get_entries(&t);
        let mut light = component.light.write().unwrap();
        let props = light.as_property_map_mut();
        for (key_type, key_name, init, range) in entries.iter() {
            if props.get(key_name).is_none() {
                let key = PropertyMap::get_key(key_type, key_name);
                props.insert(&key, init.clone());
            }
            keys.push((key_type.clone(), key_name.clone(), range.clone()));
        }
        //-------------------------------------------------------------------
        show_component_props(index, &title, ui, props, &keys, resource_selector);
    }
}
