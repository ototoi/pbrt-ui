use crate::controller::AppController;
use crate::model::scene::ResourceCacheComponent;
use crate::model::scene::ResourceComponent;

use crate::conversion::texture_node::DynaImage;
use crate::conversion::texture_node::TexturePurpose;
use crate::conversion::texture_node::create_image_variants;
use crate::conversion::texture_node::create_texture_nodes;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

//use image::DynamicImage;
use uuid::Uuid;

use eframe::egui;
use eframe::egui::Vec2;

#[derive(Debug, Clone)]
pub struct ResourcesPanel {
    pub app_controller: Arc<RwLock<AppController>>,
    pub resource_type: ResourceType,
    pub texture_id_map: HashMap<Uuid, (String, egui::TextureId, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResourceType {
    All,
    Texture,
    Material,
    Mesh,
    Other,
}

#[derive(Debug, Clone, PartialEq)]
enum IconData {
    Textured(Uuid, String, egui::TextureId),
    Colored(Uuid, String, egui::Color32),
}

impl IconData {
    pub fn get_id(&self) -> Uuid {
        match self {
            IconData::Textured(id, _, _) => *id,
            IconData::Colored(id, _, _) => *id,
        }
    }

    pub fn get_name(&self) -> String {
        match self {
            IconData::Textured(_, name, _) => name.clone(),
            IconData::Colored(_, name, _) => name.clone(),
        }
    }
}

fn short_name(name: &str, len: usize) -> String {
    let mut short_name = name.to_string();
    if short_name.len() > len {
        short_name.truncate(len);
        short_name.push_str("...");
    }
    short_name
}

fn get_image_data(image: &DynaImage) -> Option<egui::ColorImage> {
    let rgb_image = image.to_rgb8(); // Convert to RGB8 format
    let size = [rgb_image.width() as usize, rgb_image.height() as usize];
    let pixels = rgb_image.into_raw();
    Some(egui::ColorImage::from_rgb(size, &pixels))
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
        let mut icon_data = Vec::new();
        {
            let controller = self.app_controller.read().unwrap();
            let root_node = controller.get_root_node();
            let root_node = root_node.read().unwrap();
            if let Some(resource_component) = root_node.get_component::<ResourceComponent>() {
                if let Some(resource_cache_component) =
                    root_node.get_component::<ResourceCacheComponent>()
                {
                    let resource_manager = resource_component.get_resource_manager();
                    let resource_manager = resource_manager.read().unwrap();

                    if self.resource_type == ResourceType::All
                        || self.resource_type == ResourceType::Texture
                    {
                        let resource_cache_manager =
                            resource_cache_component.get_resource_cache_manager();
                        let mut resource_cache_manager = resource_cache_manager.write().unwrap();
                        create_texture_nodes(&resource_manager, &mut resource_cache_manager);
                        create_image_variants(
                            &resource_manager,
                            &mut resource_cache_manager,
                            crate::conversion::texture_node::TexturePurpose::Icon,
                        );

                        for (id, texture) in resource_manager.textures.iter() {
                            let texture = texture.read().unwrap();
                            let name = texture.get_name();
                            let edition = texture.get_edition();
                            if let Some((name, tex_id, tex_edition)) = self.texture_id_map.get(id) {
                                if edition == *tex_edition {
                                    // No need to update
                                    icon_data.push(IconData::Textured(
                                        id.clone(),
                                        name.clone(),
                                        *tex_id,
                                    ));
                                    continue;
                                }
                            }

                            if let Some(texture_node) = resource_cache_manager.textures.get(id) {
                                let texture_node = texture_node.read().unwrap();
                                if let Some(image) =
                                    texture_node.image_variants.get(&TexturePurpose::Icon)
                                {
                                    let image = image.read().unwrap();
                                    if let Some(color_image) = get_image_data(&image) {
                                        let tex_manager = ui.ctx().tex_manager();
                                        let mut tex_manager = tex_manager.write();
                                        let texture_name = format!("texture_{}_icon", id);
                                        let tex_id = tex_manager.alloc(
                                            texture_name,
                                            egui::ImageData::Color(Arc::new(color_image)),
                                            egui::TextureOptions::LINEAR,
                                        );
                                        self.texture_id_map
                                            .insert(*id, (name.clone(), tex_id, edition));
                                        icon_data.push(IconData::Textured(
                                            id.clone(),
                                            name.clone(),
                                            tex_id,
                                        ));
                                        continue;
                                    }
                                }

                                // Texture node not found
                                icon_data.push(IconData::Colored(*id, name, egui::Color32::YELLOW));
                            }
                        }
                    }

                    if self.resource_type == ResourceType::All
                        || self.resource_type == ResourceType::Material
                    {
                        for (id, res) in resource_manager.materials.iter() {
                            let res = res.read().unwrap();
                            let name = res.get_name();
                            icon_data.push(IconData::Colored(
                                id.clone(),
                                name,
                                egui::Color32::GREEN,
                            ));
                        }
                    }
                    if self.resource_type == ResourceType::All
                        || self.resource_type == ResourceType::Mesh
                    {
                        for (id, res) in resource_manager.meshes.iter() {
                            let res = res.read().unwrap();
                            let name = res.get_name();
                            icon_data.push(IconData::Colored(
                                id.clone(),
                                name,
                                egui::Color32::BLUE,
                            ));
                        }
                    }

                    if self.resource_type == ResourceType::All
                        || self.resource_type == ResourceType::Other
                    {
                        for (id, res) in resource_manager.other_resources.iter() {
                            let res = res.read().unwrap();
                            let name = res.get_name();
                            icon_data.push(IconData::Colored(
                                id.clone(),
                                name,
                                egui::Color32::GRAY,
                            ));
                        }
                    }
                }
            }
        }
        {
            if !icon_data.is_empty() {
                // Sort resources by name
                icon_data.sort_by(|a, b| a.get_name().cmp(&b.get_name()));
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
                    for icon in icon_data {
                        let id = icon.get_id();
                        let name = icon.get_name();
                        let short_name = short_name(&name, 10);
                        ui.allocate_ui(total_size, |ui| {
                            ui.vertical_centered(|ui| {
                                let (rect, response) =
                                    ui.allocate_at_least(alloc_size, egui::Sense::click_and_drag());
                                if response.hovered() {
                                    ui.painter().rect_filled(
                                        rect,
                                        1.0,
                                        egui::Color32::from_white_alpha(128),
                                    );
                                }
                                // Draw the icon here
                                match icon {
                                    IconData::Textured(_, _, texture_id) => {
                                        ui.painter().image(
                                            texture_id,
                                            rect.shrink(5.0),
                                            egui::Rect::from_min_max(
                                                egui::Pos2::new(0.0, 0.0),
                                                egui::Pos2::new(1.0, 1.0),
                                            ),
                                            egui::Color32::WHITE,
                                        );
                                    }
                                    IconData::Colored(_, _, color) => {
                                        ui.painter().rect_filled(rect.shrink(5.0), 0.0, color);
                                    }
                                }
                                if response.clicked() {
                                    // Handle click event
                                    log::info!("Clicked on resource: {}", &name);
                                    let mut controller = self.app_controller.write().unwrap();
                                    controller.set_current_resource_by_id(id);
                                }

                                ui.label(short_name);
                            });
                        });
                    }
                });
            });
    }
}
