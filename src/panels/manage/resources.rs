use crate::controllers::AppController;
use crate::controllers::texture_cache::TextureSize;
use crate::model::scene::ResourceComponent;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;
use eframe::egui::Vec2;

#[derive(Debug, Clone)]
pub struct ResourcesPanel {
    pub app_controller: Arc<RwLock<AppController>>,
    pub resource_type: ResourceType,
    pub texture_id_map: HashMap<String, egui::TextureId>,
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

fn get_image_data(path: &str) -> Option<egui::ColorImage> {
    if let Ok(image) = image::open(path) {
        let rgb_image = image.to_rgb8();
        let size = [rgb_image.width() as usize, rgb_image.height() as usize];
        let pixels = rgb_image.into_raw();
        Some(egui::ColorImage::from_rgb(size, &pixels))
    } else {
        None
    }
}

impl ResourcesPanel {
    pub fn new(controller: &Arc<RwLock<AppController>>) -> Self {
        Self {
            app_controller: controller.clone(),
            resource_type: ResourceType::All,
            texture_id_map: HashMap::new(),
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        // Here you can add more UI elements related to resources
        let mut resources = Vec::new();
        {
            let controller = self.app_controller.read().unwrap();
            let texture_cache_manager = controller.get_texture_cache_manager();

            let root_node = controller.get_root_node();
            let root_node = root_node.read().unwrap();
            if let Some(resources_component) = root_node.get_component::<ResourceComponent>() {
                let resource_manager = resources_component.get_resource_manager();
                let resource_manager = resource_manager.lock().unwrap();
                let mut texture_cache_manager = texture_cache_manager.write().unwrap();
                let texture_manager = ui.ctx().tex_manager();
                let mut texture_manager = texture_manager.write();

                if self.resource_type == ResourceType::All
                    || self.resource_type == ResourceType::Texture
                {
                    for (id, res) in resource_manager.textures.iter() {
                        let res = res.read().unwrap();
                        let name = res.get_name();
                        let texture_type = res.get_type();
                        let mut texure_id: Option<egui::TextureId> = None;
                        if texture_type == "imagemap" {
                            if let Some(fullpath) = res.get_fullpath() {
                                if let Some(cache_path) =
                                    texture_cache_manager.get_texture(&fullpath, TextureSize::Icon)
                                {
                                    //println!("Cache path: {}", cache_path);
                                    if let Some(id) = self.texture_id_map.get(&cache_path) {
                                        texure_id = Some(*id);
                                    } else {
                                        if let Some(rgb_image) = get_image_data(&cache_path) {
                                            // Create a new texture ID
                                            let color_image =
                                                egui::ImageData::Color(Arc::new(rgb_image));
                                            let texture_options = egui::TextureOptions::LINEAR;
                                            let texture_id = texture_manager.alloc(
                                                name.clone(),
                                                color_image,
                                                texture_options,
                                            );
                                            texure_id = Some(texture_id);
                                            // Store the texture ID in the map
                                            self.texture_id_map.insert(cache_path, texture_id);
                                        } else {
                                            log::error!(
                                                "Failed to load image data for: {}",
                                                cache_path
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        resources.push((id.clone(), "texture", name, texure_id));
                    }
                }
                if self.resource_type == ResourceType::All
                    || self.resource_type == ResourceType::Material
                {
                    for (id, res) in resource_manager.materials.iter() {
                        let res = res.read().unwrap();
                        let name = res.get_name();
                        resources.push((id.clone(), "material", name, None));
                    }
                }
                if self.resource_type == ResourceType::All
                    || self.resource_type == ResourceType::Mesh
                {
                    for (id, res) in resource_manager.meshes.iter() {
                        let res = res.read().unwrap();
                        let name = res.get_name();
                        resources.push((id.clone(), "mesh", name, None));
                    }
                }

                if self.resource_type == ResourceType::All
                    || self.resource_type == ResourceType::Other
                {
                    for (id, res) in resource_manager.other_resources.iter() {
                        let res = res.read().unwrap();
                        let name = res.get_name();
                        resources.push((id.clone(), "other", name, None));
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
                    for (resource_id, resource_type, resource_name, texture_id) in resources {
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

                                if let Some(texture_id) = texture_id {
                                    let uv = egui::Rect::from_min_max(
                                        egui::Pos2::new(0.0, 0.0),
                                        egui::Pos2::new(1.0, 1.0),
                                    );
                                    let icon_rect = rect.shrink(5.0);
                                    ui.painter().image(
                                        texture_id,
                                        icon_rect,
                                        uv,
                                        egui::Color32::WHITE,
                                    );
                                    //println!("Drawing texture: {:?}", texture_id);
                                } else {
                                    let icon_color = match resource_type {
                                        "texture" => egui::Color32::YELLOW,
                                        "material" => egui::Color32::GREEN,
                                        "mesh" => egui::Color32::BLUE,
                                        "other" => egui::Color32::PURPLE,
                                        _ => egui::Color32::WHITE,
                                    };
                                    let icon_rect = rect.shrink(5.0);
                                    ui.painter().rect_filled(icon_rect, 0.0, icon_color);
                                }
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
