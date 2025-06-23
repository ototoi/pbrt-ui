use super::common::*;
use super::resource_selector::ResourceSelector;
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
use crate::models::scene::MappingProperties;
use crate::models::scene::MaterialComponent;
use crate::models::scene::MaterialProperties;
use crate::models::scene::MeshComponent;
use crate::models::scene::MeshProperties;
use crate::models::scene::Node;
use crate::models::scene::OptionProperties;
use crate::models::scene::ResourceComponent;
use crate::models::scene::SamplerComponent;
use crate::models::scene::SamplerProperties;
use crate::models::scene::ShapeComponent;
use crate::models::scene::ShapeProperties;
use crate::models::scene::SubdivComponent;
use crate::models::scene::TextureProperties;
use crate::models::scene::TransformComponent;
use crate::panels::Panel;

use std::any::Any;
use std::collections::HashMap;
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
    pub material_properties: MaterialProperties,
    pub shape_properties: ShapeProperties,
    pub light_properties: LightProperties,
    pub option_properties: OptionProperties,
    pub camera_properties: CameraProperties,
    pub accelerator_properties: AcceleratorProperties,
    pub sampler_properties: SamplerProperties,
    pub integrator_properties: IntegratorProperties,
    pub texture_properties: TextureProperties,
    pub mesh_properties: MeshProperties,
    pub mapping_properties: MappingProperties,
    pub texture_id_map: Arc<RwLock<HashMap<String, egui::TextureId>>>,
}

impl InspectorPanel {
    pub fn new(controller: &Arc<RwLock<AppController>>) -> Self {
        Self {
            is_open: true,
            app_controller: controller.clone(),
            material_properties: MaterialProperties::new(),
            shape_properties: ShapeProperties::new(),
            light_properties: LightProperties::new(),
            option_properties: OptionProperties::new(),
            camera_properties: CameraProperties::new(),
            accelerator_properties: AcceleratorProperties::new(),
            sampler_properties: SamplerProperties::new(),
            integrator_properties: IntegratorProperties::new(),
            texture_properties: TextureProperties::new(),
            mesh_properties: MeshProperties::new(),
            mapping_properties: MappingProperties::new(),
            texture_id_map: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn show_inspector(&self, ui: &mut egui::Ui) {
        let controller = self.app_controller.read().unwrap();
        if let Some(node) = controller.get_current_node() {
            self.show_node(ui, &node);
        } else if let Some(current_resource) = controller.get_current_resource() {
            let id = current_resource.read().unwrap().get_id();
            self.show_resource(ui, id);
        } else {
            //ui.label("No node selected");
        }
    }

    fn get_resource_selector(&self) -> ResourceSelector {
        let controller = self.app_controller.read().unwrap();
        let root_node = controller.get_root_node();
        let root_node = root_node.read().unwrap();
        if let Some(resources_component) = root_node.get_component::<ResourceComponent>() {
            ResourceSelector::new(&resources_component.get_resource_manager())
        } else {
            ResourceSelector::default()
        }
    }

    pub fn show_node(&self, ui: &mut egui::Ui, node: &Arc<RwLock<Node>>) {
        let resource_selector = self.get_resource_selector();
        let mut node = node.write().unwrap();
        {
            let mut enabled = true; //todo
            let mut name = node.get_name();
            egui::TopBottomPanel::top("node").show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    Checkbox::without_text(&mut enabled).ui(ui);
                    ui.text_edit_singleline(&mut name);
                });
                ui.add_space(3.0);
            });
            if !name.is_empty() {
                node.set_name(&name);
            }
        }
        self.show_components(ui, &mut node.components, &resource_selector);
    }

    pub fn show_components(
        &self,
        ui: &mut egui::Ui,
        components: &mut [Box<dyn Any>],
        resource_selector: &ResourceSelector,
    ) {
        for (i, component) in components.iter_mut().enumerate() {
            if let Some(component) = component.downcast_mut::<TransformComponent>() {
                self.show_transform_component(i, ui, component, resource_selector);
            } else if let Some(component) = component.downcast_mut::<MeshComponent>() {
                self.show_mesh_component(i, ui, component);
            } else if let Some(component) = component.downcast_mut::<ShapeComponent>() {
                self.show_shape_component(i, ui, component, resource_selector);
            } else if let Some(component) = component.downcast_mut::<SubdivComponent>() {
                self.show_subdiv_component(i, ui, component, resource_selector);
            } else if let Some(component) = component.downcast_mut::<LightComponent>() {
                self.show_light_component(i, ui, component, resource_selector);
            } else if let Some(component) = component.downcast_mut::<MaterialComponent>() {
                self.show_material_component(i, ui, component, resource_selector);
            } else if let Some(component) = component.downcast_mut::<CameraComponent>() {
                self.show_typed_component(
                    i,
                    ui,
                    "Camera",
                    &mut component.props,
                    &self.camera_properties,
                    resource_selector,
                );
            } else if let Some(component) = component.downcast_mut::<FilmComponent>() {
                self.show_option_component(i, ui, "film", &mut component.props, resource_selector);
            } else if let Some(component) = component.downcast_mut::<SamplerComponent>() {
                self.show_typed_component(
                    i,
                    ui,
                    "Sampler",
                    &mut component.props,
                    &self.sampler_properties,
                    &resource_selector,
                );
            } else if let Some(component) = component.downcast_mut::<IntegratorComponent>() {
                self.show_typed_component(
                    i,
                    ui,
                    "Integrator",
                    &mut component.props,
                    &self.integrator_properties,
                    resource_selector,
                );
            } else if let Some(component) = component.downcast_mut::<AcceleratorComponent>() {
                self.show_typed_component(
                    i,
                    ui,
                    "Accelerator",
                    &mut component.props,
                    &self.accelerator_properties,
                    resource_selector,
                );
            } else if let Some(_component) = component.downcast_mut::<ResourceComponent>() {
                let mut props = PropertyMap::new();
                self.show_other_component(i, ui, "Resources", &mut props, &resource_selector);
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
        resource_selector: &ResourceSelector,
    ) {
        if let Some(mesh) = component.mesh.as_ref() {
            let mut mesh = mesh.write().unwrap();
            let props = mesh.as_property_map_mut();
            let shape_type = props.find_one_string("string type").unwrap();
            let title = shape_type.to_case(Case::Title);
            let mut keys = Vec::new();
            if let Some(params) = self.shape_properties.get(&shape_type) {
                for (key_type, key_name, init, range) in params.iter() {
                    if props.get(key_name).is_none() {
                        let key = PropertyMap::get_key(key_type, key_name);
                        props.insert(&key, init.clone());
                    }
                    keys.push((key_type.clone(), key_name.clone(), range.clone()));
                }
            }
            show_component_props(index, &title, ui, props, &keys, resource_selector);
            props.add_string("string edition", &Uuid::new_v4().to_string());
        }
    }

    fn show_subdiv_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        component: &mut SubdivComponent,
        resource_selector: &ResourceSelector,
    ) {
        if let Some(mesh) = component.mesh.as_ref() {
            let mut mesh = mesh.write().unwrap();
            let props = mesh.as_property_map_mut();
            let shape_type = props.find_one_string("string type").unwrap();
            let title = "Subdiv"; //
            let mut keys = Vec::new();
            if let Some(params) = self.shape_properties.get(&shape_type) {
                for (key_type, key_name, init, range) in params.iter() {
                    if props.get(key_name).is_none() {
                        let key = PropertyMap::get_key(key_type, key_name);
                        props.insert(&key, init.clone());
                    }
                    keys.push((key_type.clone(), key_name.clone(), range.clone()));
                }
            }
            show_component_props(index, &title, ui, props, &keys, resource_selector);
            //props.add_string("string edition", &Uuid::new_v4().to_string());
        }
    }

    fn show_option_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        option_type: &str,
        props: &mut PropertyMap,
        resource_selector: &ResourceSelector,
    ) {
        let title = option_type.to_case(Case::Title);
        let mut keys = Vec::new();
        if let Some(params) = self.option_properties.get(option_type) {
            for (key_type, key_name, init) in params.iter() {
                if props.get(key_name).is_none() {
                    let key = PropertyMap::get_key(key_type, key_name);
                    props.insert(&key, init.clone());
                }
                keys.push((key_type.clone(), key_name.clone(), None)); //todo: add range
            }
        }
        show_component_props(index, &title, ui, props, &keys, resource_selector);
    }

    fn show_other_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        title: &str,
        props: &mut PropertyMap,
        resource_selector: &ResourceSelector,
    ) {
        let keys = props.get_keys();
        let keys = keys
            .iter()
            .map(|(key_type, key_name)| (key_type.clone(), key_name.clone(), None))
            .collect::<Vec<_>>();
        show_component_props(index, title, ui, props, &keys, resource_selector);
    }

    pub fn show_resource(&self, ui: &mut egui::Ui, id: Uuid) {
        let controller = self.app_controller.read().unwrap();
        let root_node = controller.get_root_node();
        let root_node = root_node.read().unwrap();

        if let Some(resources_component) = root_node.get_component::<ResourceComponent>() {
            let resource_manager = resources_component.get_resource_manager();
            let resource_selector = ResourceSelector::new(&resource_manager);
            let resource_manager = resource_manager.lock().unwrap();
            if let Some(texture) = resource_manager.textures.get(&id) {
                let mut texture = texture.write().unwrap();
                let mut name = texture.get_name();

                let props = texture.as_property_map_mut();
                let t = props.find_one_string("string type").unwrap();
                let mut texture_keys = Vec::new();
                if let Some(params) = self.texture_properties.get(&t) {
                    for (key_type, key_name, init, range) in params.iter() {
                        if props.get(key_name).is_none() {
                            let key = PropertyMap::get_key(key_type, key_name);
                            props.insert(&key, init.clone());
                        }
                        texture_keys.push((key_type.clone(), key_name.clone(), range.clone()));
                    }
                }
                let mapping = props
                    .find_one_string("string mapping")
                    .unwrap_or("uv".to_string());
                let mut mapping_keys = Vec::new();
                if let Some(params) = self.mapping_properties.get(&mapping) {
                    for (key_type, key_name, init, range) in params.iter() {
                        if props.get(key_name).is_none() {
                            let key = PropertyMap::get_key(key_type, key_name);
                            props.insert(&key, init.clone());
                        }
                        mapping_keys.push((key_type.clone(), key_name.clone(), range.clone()));
                    }
                }
                //---------------------------------------------------------------------
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label("Texture");
                    ui.separator();
                    ui.text_edit_singleline(&mut name);
                });
                ui.separator();
                show_type(ui, props, &[t.clone()]);
                ui.separator();
                self.show_texture_preview(ui, 300.0, props);
                ui.separator();
                show_properties(0, ui, props, &texture_keys, &resource_selector);
                show_properties(1, ui, props, &mapping_keys, &resource_selector);
                ui.add_space(3.0);
            } else if let Some(material) = resource_manager.materials.get(&id) {
                let mut material = material.write().unwrap();
                let mut name = material.get_name();

                let props = material.as_property_map_mut();
                let t = props.find_one_string("string type").unwrap();

                let mut keys = Vec::new();
                if let Some(params) = self.material_properties.get(&t) {
                    for (key_type, key_name, init, range) in params.iter() {
                        if props.get(key_name).is_none() {
                            let key = PropertyMap::get_key(key_type, key_name);
                            props.insert(&key, init.clone());
                        }
                        keys.push((key_type.clone(), key_name.clone(), range.clone()));
                    }
                }
                //---------------------------------------------------------------------
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label("Material");
                    ui.separator();
                    ui.text_edit_singleline(&mut name);
                });
                ui.separator();
                show_type(ui, props, &self.material_properties.get_types());
                ui.separator();
                self.show_material_preview(ui, 300.0, props);
                ui.separator();
                show_properties(0, ui, props, &keys, &resource_selector);
                ui.add_space(3.0);
            } else if let Some(mesh) = resource_manager.meshes.get(&id) {
                let mut mesh = mesh.write().unwrap();
                let mut name = mesh.get_name();
                let props = mesh.as_property_map_mut();
                let t = props.find_one_string("string type").unwrap();
                let mut keys = Vec::new();
                if let Some(params) = self.mesh_properties.get(&t) {
                    for (key_type, key_name, init, range) in params.iter() {
                        if props.get(key_name).is_none() {
                            let key = PropertyMap::get_key(key_type, key_name);
                            props.insert(&key, init.clone());
                        }
                        keys.push((key_type.clone(), key_name.clone(), range.clone()));
                    }
                }
                //---------------------------------------------------------------------
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label("Mesh");
                    ui.separator();
                    ui.text_edit_singleline(&mut name);
                });
                ui.separator();
                self.show_mesh_preview(ui, 300.0, props);
                ui.separator();
                show_properties(0, ui, props, &keys, &resource_selector);
                ui.add_space(3.0);
            } else if let Some(res) = resource_manager.other_resources.get(&id) {
                let res = res.write().unwrap();
                let mut name = res.get_name();
                let t = res.get_type();
                let filename = res.get_filename().unwrap_or_default();
                let mut props = PropertyMap::new();
                props.add_string("string type", &t);
                props.add_string("string filename", &filename);

                let mut keys = Vec::new();
                //keys.push(("string".to_string(), "type".to_string(), None));
                keys.push(("string".to_string(), "filename".to_string(), None));
                //---------------------------------------------------------------------
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label(&t);
                    ui.separator();
                    ui.text_edit_singleline(&mut name);
                });
                ui.separator();
                //show_type(ui, &mut props, &[t.clone()]);
                //ui.separator();
                show_properties(0, ui, &mut props, &keys, &resource_selector);
                ui.add_space(3.0);
            } else {
                ui.label("Resource not found");
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
            .max_width(600.0)
            .resizable(true)
            .show_animated(ctx, self.is_open, |ui| {
                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("X").clicked() {
                            self.toggle_open();
                        }
                        ui.vertical_centered(|ui| {
                            ui.label("Inspector");
                        });
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        self.show_inspector(ui);
                    });
            });
    }
}
