use super::common::*;
use crate::controllers::AppController;
use crate::models::base::PropertyMap;
use crate::models::scene::AcceleratorComponent;
use crate::models::scene::AcceleratorProperties;
use crate::models::scene::CameraComponent;
use crate::models::scene::CameraProperties;
use crate::models::scene::FilmComponent;
use crate::models::scene::IntegratorComponent;
use crate::models::scene::IntegratorProperties;
use crate::models::scene::LightComponent;
use crate::models::scene::LightProperties;
use crate::models::scene::MaterialComponent;
use crate::models::scene::MaterialProperties;
use crate::models::scene::MeshComponent;
use crate::models::scene::MeshProperties;
use crate::models::scene::OptionProperties;
use crate::models::scene::ResourcesComponent;
use crate::models::scene::SamplerComponent;
use crate::models::scene::SamplerProperties;
use crate::models::scene::ShapeComponent;
use crate::models::scene::ShapeProperties;
use crate::models::scene::SubdivComponent;
use crate::models::scene::TextureProperties;
use crate::models::scene::TransformComponent;
use crate::panels::Panel;

use std::any::Any;
use std::sync::Arc;
use std::sync::RwLock;

use convert_case::{Case, Casing};
use eframe::egui;
use eframe::egui::Checkbox;
use eframe::egui::Widget;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct InspectorPanel {
    pub is_open: bool,
    pub app_controller: Arc<RwLock<AppController>>,
    pub material_parameters: MaterialProperties,
    pub shape_parameters: ShapeProperties,
    pub light_parameters: LightProperties,
    pub option_parameters: OptionProperties,
    pub camera_parameters: CameraProperties,
    pub accelerator_parameters: AcceleratorProperties,
    pub sampler_parameters: SamplerProperties,
    pub integrator_parameters: IntegratorProperties,
    pub texture_parameters: TextureProperties,
    pub mesh_parameters: MeshProperties,
}

impl InspectorPanel {
    pub fn new(controller: &Arc<RwLock<AppController>>) -> Self {
        Self {
            is_open: true,
            app_controller: controller.clone(),
            material_parameters: MaterialProperties::new(),
            shape_parameters: ShapeProperties::new(),
            light_parameters: LightProperties::new(),
            option_parameters: OptionProperties::new(),
            camera_parameters: CameraProperties::new(),
            accelerator_parameters: AcceleratorProperties::new(),
            sampler_parameters: SamplerProperties::new(),
            integrator_parameters: IntegratorProperties::new(),
            texture_parameters: TextureProperties::new(),
            mesh_parameters: MeshProperties::new(),
        }
    }

    pub fn show_node(&self, ui: &mut egui::Ui) {
        let controller = self.app_controller.read().unwrap();
        if let Some(current_node) = controller.get_current_node() {
            let mut current_node = current_node.write().unwrap();
            {
                let mut enabled = true; //todo
                let mut name = current_node.get_name();
                egui::TopBottomPanel::top("node").show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        Checkbox::without_text(&mut enabled).ui(ui);
                        ui.text_edit_singleline(&mut name);
                    });
                    ui.add_space(3.0);
                });
                if !name.is_empty() {
                    current_node.set_name(&name);
                }
            }
            self.show_components(ui, &mut current_node.components);
        } else if let Some(current_resource) = controller.get_current_resource() {
            let id = current_resource.read().unwrap().get_id();
            self.show_resource(ui, id);
        } else {
            //ui.label("No node selected");
        }
    }

    pub fn show_components(&self, ui: &mut egui::Ui, components: &mut [Box<dyn Any>]) {
        for (i, component) in components.iter_mut().enumerate() {
            if let Some(component) = component.downcast_mut::<TransformComponent>() {
                self.show_transform_component(i, ui, component);
            } else if let Some(component) = component.downcast_mut::<MeshComponent>() {
                self.show_mesh_component(i, ui, component);
            } else if let Some(component) = component.downcast_mut::<ShapeComponent>() {
                self.show_shape_component(i, ui, component);
            } else if let Some(component) = component.downcast_mut::<SubdivComponent>() {
                self.show_subdiv_component(i, ui, component);
            } else if let Some(component) = component.downcast_mut::<LightComponent>() {
                self.show_light_component(i, ui, component);
            } else if let Some(component) = component.downcast_mut::<MaterialComponent>() {
                self.show_material_component(i, ui, component);
            } else if let Some(component) = component.downcast_mut::<CameraComponent>() {
                self.show_typed_component(
                    i,
                    ui,
                    "Camera",
                    &mut component.props,
                    &self.camera_parameters,
                );
            } else if let Some(component) = component.downcast_mut::<FilmComponent>() {
                self.show_option_component(i, ui, "film", &mut component.props);
            } else if let Some(component) = component.downcast_mut::<SamplerComponent>() {
                self.show_typed_component(
                    i,
                    ui,
                    "Sampler",
                    &mut component.props,
                    &self.sampler_parameters,
                );
            } else if let Some(component) = component.downcast_mut::<IntegratorComponent>() {
                self.show_typed_component(
                    i,
                    ui,
                    "Integrator",
                    &mut component.props,
                    &self.integrator_parameters,
                );
            } else if let Some(component) = component.downcast_mut::<AcceleratorComponent>() {
                self.show_typed_component(
                    i,
                    ui,
                    "Accelerator",
                    &mut component.props,
                    &self.accelerator_parameters,
                );
            } else if let Some(_component) = component.downcast_mut::<ResourcesComponent>() {
                let mut props = PropertyMap::new();
                self.show_other_component(i, ui, "Resources", &mut props);
            } else {
                //log::warn!("Unknown component type");
            }
        }
    }

    fn show_shape_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        component: &mut ShapeComponent,
    ) {
        if let Some(mesh) = component.mesh.as_ref() {
            let mut mesh = mesh.write().unwrap();
            let props = mesh.as_property_map_mut();
            let shape_type = props.find_one_string("string type").unwrap();
            let title = shape_type.to_case(Case::Title);
            let mut keys = Vec::new();
            if let Some(params) = self.shape_parameters.get(&shape_type) {
                for (key_type, key_name, init, range) in params.iter() {
                    if props.get(key_name).is_none() {
                        let key = PropertyMap::get_key(key_type, key_name);
                        props.insert(&key, init.clone());
                    }
                    keys.push((key_type.clone(), key_name.clone(), range.clone()));
                }
            }
            show_component_props(index, &title, ui, props, &keys);
            props.add_string("string edition", &Uuid::new_v4().to_string());
        }
    }

    fn show_subdiv_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        component: &mut SubdivComponent,
    ) {
        if let Some(mesh) = component.mesh.as_ref() {
            let mut mesh = mesh.write().unwrap();
            let props = mesh.as_property_map_mut();
            let shape_type = props.find_one_string("string type").unwrap();
            let title = "Subdiv"; //
            let mut keys = Vec::new();
            if let Some(params) = self.shape_parameters.get(&shape_type) {
                for (key_type, key_name, init, range) in params.iter() {
                    if props.get(key_name).is_none() {
                        let key = PropertyMap::get_key(key_type, key_name);
                        props.insert(&key, init.clone());
                    }
                    keys.push((key_type.clone(), key_name.clone(), range.clone()));
                }
            }
            show_component_props(index, &title, ui, props, &keys);
            //props.add_string("string edition", &Uuid::new_v4().to_string());
        }
    }

    fn show_option_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        option_type: &str,
        props: &mut PropertyMap,
    ) {
        let title = option_type.to_case(Case::Title);
        let mut keys = Vec::new();
        if let Some(params) = self.option_parameters.get(option_type) {
            for (key_type, key_name, init) in params.iter() {
                if props.get(key_name).is_none() {
                    let key = PropertyMap::get_key(key_type, key_name);
                    props.insert(&key, init.clone());
                }
                keys.push((key_type.clone(), key_name.clone(), None)); //todo: add range
            }
        }
        show_component_props(index, &title, ui, props, &keys);
    }

    fn show_other_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        title: &str,
        props: &mut PropertyMap,
    ) {
        let keys = props.get_keys();
        let keys = keys
            .iter()
            .map(|(key_type, key_name)| (key_type.clone(), key_name.clone(), None))
            .collect::<Vec<_>>();
        show_component_props(index, title, ui, props, &keys);
    }

    pub fn show_resource(&self, ui: &mut egui::Ui, id: Uuid) {
        let controller = self.app_controller.read().unwrap();
        let root_node = controller.get_root_node();
        let root_node = root_node.read().unwrap();

        if let Some(resources_component) = root_node.get_component::<ResourcesComponent>() {
            if let Some(texture) = resources_component.textures.get(&id) {
                let mut texture = texture.write().unwrap();
                let name = texture.get_name();
                let props = texture.as_property_map_mut();
                let t = props.find_one_string("string type").unwrap();
                let title = format!("Texture: {}", name); //
                let mut keys = Vec::new();
                if let Some(params) = self.texture_parameters.get(&t) {
                    for (key_type, key_name, init, range) in params.iter() {
                        if props.get(key_name).is_none() {
                            let key = PropertyMap::get_key(key_type, key_name);
                            props.insert(&key, init.clone());
                        }
                        keys.push((key_type.clone(), key_name.clone(), range.clone()));
                    }
                }
                show_resource_props(0, &title, ui, props, &keys);
            } else if let Some(material) = resources_component.materials.get(&id) {
                let mut material = material.write().unwrap();
                let name = material.get_name();
                let props = material.as_property_map_mut();
                let t = props.find_one_string("string type").unwrap();
                let title = format!("Material: {}", name); //
                let mut keys = Vec::new();
                if let Some(params) = self.material_parameters.get(&t) {
                    for (key_type, key_name, init, range) in params.iter() {
                        if props.get(key_name).is_none() {
                            let key = PropertyMap::get_key(key_type, key_name);
                            props.insert(&key, init.clone());
                        }
                        keys.push((key_type.clone(), key_name.clone(), range.clone()));
                    }
                }
                show_resource_props(0, &title, ui, props, &keys);
            } else if let Some(mesh) = resources_component.meshes.get(&id) {
                let mut mesh = mesh.write().unwrap();
                let name = mesh.get_name();
                let props = mesh.as_property_map_mut();
                let t = props.find_one_string("string type").unwrap();
                let title = format!("Mesh: {}", name); //
                let mut keys = Vec::new();
                if let Some(params) = self.mesh_parameters.get(&t) {
                    for (key_type, key_name, init, range) in params.iter() {
                        if props.get(key_name).is_none() {
                            let key = PropertyMap::get_key(key_type, key_name);
                            props.insert(&key, init.clone());
                        }
                        keys.push((key_type.clone(), key_name.clone(), range.clone()));
                    }
                }
                show_resource_props(0, &title, ui, props, &keys);
            }
        }
    }
}

impl Panel for InspectorPanel {
    fn name(&self) -> &str {
        "Inspector"
    }
    fn is_open(&self) -> bool {
        self.is_open
    }
    fn toggle_open(&mut self) -> bool {
        self.is_open = !self.is_open;
        self.is_open
    }
    fn show(&mut self, ctx: &egui::Context) {
        egui::SidePanel::right("inspector")
            .default_width(450.0)
            .min_width(200.0)
            .max_width(450.0)
            .resizable(true)
            .show_animated(ctx, self.is_open, |ui| {
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("X").clicked() {
                            self.toggle_open();
                        }
                        ui.vertical_centered(|ui| {
                            ui.label("Hierarchy");
                        });
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        self.show_node(ui);
                    });
            });
    }
}
