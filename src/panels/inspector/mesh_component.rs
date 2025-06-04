use super::common::*;
use super::panel::InspectorPanel;
use crate::models::scene::MeshComponent;

use eframe::egui;

impl InspectorPanel {
    pub fn show_mesh_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        component: &mut MeshComponent,
    ) {
        if let Some(mesh) = &component.mesh {
            //let mesh = mesh.read().unwrap();
            //show_component_props(index, "Mesh", ui, &mut mesh.props, &keys);
        }
    }
}
