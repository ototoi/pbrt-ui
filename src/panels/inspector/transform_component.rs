use super::common::*;
use super::panel::InspectorPanel;
use crate::model::scene::TransformComponent;
use crate::panels::inspector::resource_selector::{self, ResourceSelector};

use eframe::egui;

impl InspectorPanel {
    pub fn show_transform_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        component: &mut TransformComponent,
        resource_selector: &ResourceSelector,
    ) {
        let keys = vec![
            ("float".to_string(), "position".to_string(), None),
            ("float".to_string(), "rotation".to_string(), None),
            ("float".to_string(), "scale".to_string(), None),
        ];
        show_component_props(
            index + 1,
            "Transform",
            ui,
            &mut component.props,
            &keys,
            resource_selector,
        );
    }
}
