use super::common::*;
use super::panel::InspectorPanel;
use crate::model::scene::TransformComponent;
use crate::panel::inspector::resource_selector::ResourceSelector;

use eframe::egui;
use uuid::Uuid;

impl InspectorPanel {
    pub fn show_transform_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        component: &mut TransformComponent,
        resource_selector: &ResourceSelector,
    ) -> bool {
        let keys = vec![
            ("float".to_string(), "position".to_string(), None),
            ("float".to_string(), "rotation".to_string(), None),
            ("float".to_string(), "scale".to_string(), None),
        ];
        let mut is_changed = false;
        if show_component_props(
            index + 1,
            "Transform",
            ui,
            &mut component.props,
            &keys,
            resource_selector,
        ) {
            is_changed = true;
            component
                .props
                .add_string("string edition", &Uuid::new_v4().to_string());
        }
        return is_changed;
    }
}
