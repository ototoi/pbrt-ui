use super::common::*;
use super::resource_selector::ResourceSelector;
use crate::controller::AppController;
use crate::model::base::PropertyMap;
use crate::model::scene::AcceleratorComponent;
use crate::model::scene::AcceleratorProperties;
use crate::model::scene::AnimationComponent;
use crate::model::scene::CameraComponent;
use crate::model::scene::CameraProperties;
use crate::model::scene::CoordinateSystemComponent;
use crate::model::scene::FilmComponent;
use crate::model::scene::IntegratorComponent;
use crate::model::scene::IntegratorProperties;
use crate::model::scene::LightComponent;
use crate::model::scene::LightProperties;
use crate::model::scene::MappingProperties;
use crate::model::scene::MaterialComponent;
use crate::model::scene::MaterialProperties;
use crate::model::scene::Node;
use crate::model::scene::OptionProperties;
use crate::model::scene::Properties;
use crate::model::scene::ResourceComponent;
use crate::model::scene::SamplerComponent;
use crate::model::scene::SamplerProperties;
use crate::model::scene::ShapeComponent;
use crate::model::scene::ShapeProperties;
use crate::model::scene::TextureProperties;
use crate::model::scene::TransformComponent;
use crate::panel::Panel;

use std::any::Any;
use std::collections::HashMap;
use std::f32::consts::E;
use std::sync::Arc;
use std::sync::RwLock;

use convert_case::{Case, Casing};
use eframe::egui;
use eframe::egui::Checkbox;
use eframe::egui::Widget;
use tar::Entry;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct InspectorPanel {
    pub is_open: bool,
    pub app_controller: Arc<RwLock<AppController>>,
    pub texture_id_map: Arc<RwLock<HashMap<Uuid, (String, egui::TextureId)>>>,
}

impl InspectorPanel {
    pub fn new(controller: &Arc<RwLock<AppController>>) -> Self {
        Self {
            is_open: true,
            app_controller: controller.clone(),
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
            let mut enabled = node.is_enabled();
            let mut name = node.get_name();
            egui::TopBottomPanel::top("node").show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    Checkbox::without_text(&mut enabled).ui(ui);
                    ui.text_edit_singleline(&mut name);
                });
                ui.add_space(3.0);
            });
            node.set_enable(enabled);
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
            } else if let Some(component) = component.downcast_mut::<ShapeComponent>() {
                self.show_shape_component(i, ui, component, resource_selector);
            } else if let Some(component) = component.downcast_mut::<LightComponent>() {
                self.show_light_component(i, ui, component, resource_selector);
            } else if let Some(component) = component.downcast_mut::<MaterialComponent>() {
                self.show_material_component(i, ui, component, resource_selector);
            } else if let Some(component) = component.downcast_mut::<CameraComponent>() {
                let camera_properties = CameraProperties::get_instance();
                self.show_typed_component(
                    i,
                    ui,
                    "Camera",
                    &mut component.props,
                    &(*camera_properties),
                    resource_selector,
                );
            } else if let Some(component) = component.downcast_mut::<FilmComponent>() {
                self.show_option_component(i, ui, "film", &mut component.props, resource_selector);
            } else if let Some(component) = component.downcast_mut::<SamplerComponent>() {
                let sampler_properties = SamplerProperties::get_instance();
                self.show_typed_component(
                    i,
                    ui,
                    "Sampler",
                    &mut component.props,
                    &(*sampler_properties),
                    &resource_selector,
                );
            } else if let Some(component) = component.downcast_mut::<IntegratorComponent>() {
                let integrator_properties = IntegratorProperties::get_instance();
                self.show_typed_component(
                    i,
                    ui,
                    "Integrator",
                    &mut component.props,
                    &(*integrator_properties),
                    resource_selector,
                );
            } else if let Some(component) = component.downcast_mut::<AcceleratorComponent>() {
                let accelerator_properties = AcceleratorProperties::get_instance();
                self.show_typed_component(
                    i,
                    ui,
                    "Accelerator",
                    &mut component.props,
                    &(*accelerator_properties),
                    resource_selector,
                );
            } else if let Some(component) = component.downcast_mut::<CoordinateSystemComponent>() {
                let mut props = PropertyMap::new();
                let up = component.get_up_vector();
                props.add_floats("float up", &[up.x, up.y, up.z]);
                self.show_other_component(
                    i,
                    ui,
                    "CoordinateSystem",
                    &mut props,
                    &resource_selector,
                );
            } else if let Some(_component) = component.downcast_mut::<AnimationComponent>() {
                let mut props = PropertyMap::new(); //todo
                show_component_props(i, "Animation", ui, &mut props, &[], resource_selector);
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
    ) -> bool {
        const IGNORE_KEYS: [&str; 5] = ["indices", "P", "N", "S", "uv"];
        let mut is_changed = false;
        let shape = component.get_shape();
        let mut shape = shape.write().unwrap();
        let props = shape.as_property_map_mut();
        let shape_type = props.find_one_string("string type").unwrap();
        let name = props.find_one_string("string name").unwrap();
        let mut keys = Vec::new();
        let shape_properties = ShapeProperties::get_instance();
        if let Some(params) = shape_properties.get_entries(&shape_type) {
            for entry in params.iter() {
                if IGNORE_KEYS.contains(&entry.key_name.as_str()) {
                    continue; // Skip keys that should not be shown
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
        }
        if show_component_props(index, &name, ui, props, &keys, resource_selector) {
            if ShapeComponent::is_ediable(&shape_type) {
                is_changed = true;
                props.add_string("string edition", &Uuid::new_v4().to_string());
            }
        }
        return is_changed;
    }

    fn show_option_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        option_type: &str,
        props: &mut PropertyMap,
        resource_selector: &ResourceSelector,
    ) -> bool {
        let mut is_changed = false;
        let title = option_type.to_case(Case::Title);
        let mut keys = Vec::new();
        let option_properties = OptionProperties::get_instance();
        if let Some(params) = option_properties.get_entries(option_type) {
            for entry in params.iter() {
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
        }
        if show_component_props(index, &title, ui, props, &keys, resource_selector) {
            is_changed = true;
            props.add_string("string edition", &Uuid::new_v4().to_string());
        }
        return is_changed;
    }

    fn show_other_component(
        &self,
        index: usize,
        ui: &mut egui::Ui,
        title: &str,
        props: &mut PropertyMap,
        resource_selector: &ResourceSelector,
    ) -> bool {
        let mut is_changed = false;
        let keys = props.get_keys();
        let keys = keys
            .iter()
            .map(|(key_type, key_name)| (key_type.clone(), key_name.clone(), None))
            .collect::<Vec<_>>();
        if show_component_props(index, title, ui, props, &keys, resource_selector) {
            is_changed = true;
            props.add_string("string edition", &Uuid::new_v4().to_string());
        }
        return is_changed;
    }

    pub fn show_resource(&self, ui: &mut egui::Ui, id: Uuid) {
        let controller = self.app_controller.read().unwrap();
        let root_node = controller.get_root_node();
        let root_node = root_node.read().unwrap();

        if let Some(resources_component) = root_node.get_component::<ResourceComponent>() {
            let resource_manager = resources_component.get_resource_manager();
            let resource_selector = ResourceSelector::new(&resource_manager);
            let resource_manager = resource_manager.read().unwrap();
            if let Some(texture) = resource_manager.textures.get(&id) {
                let mut texture_keys = Vec::new();
                let mut mapping_keys = Vec::new();
                {
                    let mut texture = texture.write().unwrap();

                    let props = texture.as_property_map_mut();
                    let t = props.find_one_string("string type").unwrap();

                    let texture_properties = TextureProperties::get_instance();
                    if let Some(params) = texture_properties.get_entries(&t) {
                        for entry in params.iter() {
                            if props.get(&entry.key_name).is_none() {
                                let key = PropertyMap::get_key(&entry.key_type, &entry.key_name);
                                props.insert(&key, entry.default_value.clone());
                            }
                            texture_keys.push((
                                entry.key_type.clone(),
                                entry.key_name.clone(),
                                entry.value_range.clone(),
                            ));
                        }
                    }
                    let mapping = props
                        .find_one_string("string mapping")
                        .unwrap_or("uv".to_string());

                    let mapping_properties = MappingProperties::get_instance();
                    if let Some(params) = mapping_properties.get_entries(&mapping) {
                        for entry in params.iter() {
                            if props.get(&entry.key_name).is_none() {
                                let key = PropertyMap::get_key(&entry.key_type, &entry.key_name);
                                props.insert(&key, entry.default_value.clone());
                            }
                            mapping_keys.push((
                                entry.key_type.clone(),
                                entry.key_name.clone(),
                                entry.value_range.clone(),
                            ));
                        }
                    }
                }
                //---------------------------------------------------------------------
                ui.add_space(2.0);
                {
                    let texture = texture.read().unwrap();
                    let mut name = texture.get_name();
                    ui.horizontal(|ui| {
                        ui.label("Texture");
                        ui.separator();
                        ui.text_edit_singleline(&mut name);
                    });
                }
                ui.separator();
                {
                    let mut texture = texture.write().unwrap();
                    let t = texture.get_type();
                    let props = texture.as_property_map_mut();
                    show_type(ui, props, &[t.clone()]);
                }
                ui.separator();
                {
                    let texture = texture.read().unwrap();
                    self.show_texture_preview(ui, 300.0, &texture);
                }
                ui.separator();
                {
                    let mut texture = texture.write().unwrap();
                    let props = texture.as_property_map_mut();
                    show_properties(0, ui, props, &texture_keys, &resource_selector);
                    show_properties(1, ui, props, &mapping_keys, &resource_selector);
                }
                ui.add_space(3.0);
            } else if let Some(material) = resource_manager.materials.get(&id) {
                let mut material = material.write().unwrap();
                let mut name = material.get_name();

                let props = material.as_property_map_mut();
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
                let mut keys = Vec::new();
                let material_properties = MaterialProperties::get_instance();
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
                }
                //---------------------------------------------------------------------
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label("Material");
                    ui.separator();
                    ui.text_edit_singleline(&mut name);
                });
                ui.separator();
                show_type(ui, props, &material_properties.get_types());
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
                let shape_properties = ShapeProperties::get_instance();
                if let Some(params) = shape_properties.get_entries(&t) {
                    for entry in params.iter() {
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
