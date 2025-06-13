use crate::controllers::AppController;
use crate::models::scene::ResourceComponent;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;
use eframe::egui::Vec2;

#[derive(Debug, Clone)]
pub struct ResourcesPanel {
    pub app_controller: Arc<RwLock<AppController>>,
    pub resource_type: ResourceType,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResourceType {
    All,
    Texture,
    Material,
    Mesh,
    Other,
}

fn short_name(name: &str, len: usize) -> String {
    let mut short_name = name.to_string();
    if short_name.len() > len {
        short_name.truncate(len);
        short_name.push_str("...");
    }
    short_name
}

impl ResourcesPanel {
    pub fn new(controller: &Arc<RwLock<AppController>>) -> Self {
        Self {
            app_controller: controller.clone(),
            resource_type: ResourceType::All,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        // Here you can add more UI elements related to resources
        let mut resources = Vec::new();
        {
            let controller = self.app_controller.read().unwrap();
            let root_node = controller.get_root_node();
            let root_node = root_node.read().unwrap();
            if let Some(resources_component) = root_node.get_component::<ResourceComponent>() {
                if self.resource_type == ResourceType::All
                    || self.resource_type == ResourceType::Texture
                {
                    for (id, res) in resources_component.textures.iter() {
                        let res = res.read().unwrap();
                        let name = res.get_name();
                        resources.push((id.clone(), "texture", name));
                    }
                }
                if self.resource_type == ResourceType::All
                    || self.resource_type == ResourceType::Material
                {
                    for (id, res) in resources_component.materials.iter() {
                        let res = res.read().unwrap();
                        let name = res.get_name();
                        resources.push((id.clone(), "material", name));
                    }
                }
                if self.resource_type == ResourceType::All
                    || self.resource_type == ResourceType::Mesh
                {
                    for (id, res) in resources_component.meshes.iter() {
                        let res = res.read().unwrap();
                        let name = res.get_name();
                        resources.push((id.clone(), "mesh", name));
                    }
                }

                if self.resource_type == ResourceType::All
                    || self.resource_type == ResourceType::Other
                {
                    for (id, res) in resources_component.other_resources.iter() {
                        let res = res.read().unwrap();
                        let name = res.get_name();
                        resources.push((id.clone(), "other", name));
                    }
                }
            }
        }
        {
            if !resources.is_empty() {
                // Sort resources by name
                resources.sort_by(|a, b| a.2.cmp(&b.2));
            }
        }
        egui::SidePanel::left("resources_type")
            .default_width(200.0)
            .resizable(true)
            .show_inside(ui, |ui| {
                ui.radio_value(&mut self.resource_type, ResourceType::All, "All");
                ui.radio_value(&mut self.resource_type, ResourceType::Texture, "Textures");
                ui.radio_value(&mut self.resource_type, ResourceType::Material, "Materials");
                ui.radio_value(&mut self.resource_type, ResourceType::Mesh, "Meshes");
                ui.radio_value(&mut self.resource_type, ResourceType::Other, "Others");
            });

        let icon_size = Vec2::new(80.0, 80.0);
        let alloc_size = icon_size + Vec2::new(5.0, 5.0);
        let total_size = alloc_size + Vec2::new(0.0, 20.0);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (resource_id, resource_type, resource_name) in resources {
                        let short_name = short_name(&resource_name, 10);
                        ui.allocate_ui(total_size, |ui| {
                            ui.vertical_centered(|ui| {
                                let (rect, response) =
                                    ui.allocate_at_least(alloc_size, egui::Sense::click_and_drag());
                                if response.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        1.0,
                                        egui::Color32::from_white_alpha(10),
                                    );
                                }
                                // Draw the icon here
                                /*
                                ui.painter().rect_stroke(
                                    rect,
                                    10.0,
                                    egui::Stroke::new(1.0, egui::Color32::RED),
                                    egui::StrokeKind::Outside
                                );
                                */
                                let icon_color = match resource_type {
                                    "texture" => egui::Color32::YELLOW,
                                    "material" => egui::Color32::GREEN,
                                    "mesh" => egui::Color32::BLUE,
                                    "other" => egui::Color32::PURPLE,
                                    _ => egui::Color32::WHITE,
                                };
                                let icon_rect = rect.shrink(5.0);
                                ui.painter().rect_filled(icon_rect, 5.0, icon_color);
                                if response.clicked() {
                                    // Handle click event
                                    log::info!("Clicked on resource: {}", &resource_name);
                                    let mut controller = self.app_controller.write().unwrap();
                                    controller.set_current_resource_by_id(resource_id);
                                }
                                ui.label(short_name);
                            });
                        });
                    }
                });
            });
    }
}
