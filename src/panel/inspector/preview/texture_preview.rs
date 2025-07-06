use super::super::panel::InspectorPanel;
//use super::super::common::*;
use crate::geometry::texture_cache::TextureCacheManager;
use crate::geometry::texture_cache::TextureSize;
use crate::model::base::*;
use crate::model::scene::Node;
use crate::model::scene::ResourceCacheComponent;

use std::sync::{Arc, RwLock};

use eframe::egui;

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

fn get_texture_cache_manager(node: &Arc<RwLock<Node>>) -> Option<Arc<RwLock<TextureCacheManager>>> {
    let node = node.read().unwrap();
    if let Some(resource_cache) = node.get_component::<ResourceCacheComponent>() {
        Some(resource_cache.get_texture_cache_manager())
    } else {
        None
    }
}

impl InspectorPanel {
    pub fn show_texture_preview(&self, ui: &mut egui::Ui, width: f32, props: &mut PropertyMap) {
        let mut texture_id = None;
        let texture_type = props
            .find_one_string("string type")
            .unwrap_or("".to_string());
        if texture_type == "imagemap" {
            let name = props
                .find_one_string("string name")
                .unwrap_or("".to_string());
            let fullpath = props
                .find_one_string("string fullpath")
                .unwrap_or("".to_string());
            if !fullpath.is_empty() {
                let controller = self.app_controller.read().unwrap();
                let root_node = controller.get_root_node();
                if let Some(texture_cache_manager) = get_texture_cache_manager(&root_node) {
                    let mut texture_cache_manager = texture_cache_manager.write().unwrap();
                    if let Some(cache_path) =
                        texture_cache_manager.get_texture(&fullpath, TextureSize::Full)
                    {
                        let mut texture_id_map = self.texture_id_map.write().unwrap();
                        if let Some(id) = texture_id_map.get(&cache_path) {
                            texture_id = Some(*id);
                        } else {
                            if let Some(rgb_image) = get_image_data(&cache_path) {
                                let texture_manager = ui.ctx().tex_manager();
                                let mut texture_manager = texture_manager.write();
                                // Create a new texture ID
                                let color_image = egui::ImageData::Color(Arc::new(rgb_image));
                                let texture_options = egui::TextureOptions::LINEAR;
                                let id = texture_manager.alloc(
                                    name.clone(),
                                    color_image,
                                    texture_options,
                                );
                                texture_id = Some(id);
                                // Store the texture ID in the map
                                texture_id_map.insert(cache_path, id);
                            }
                        }
                    }
                }
            }
        }

        let width = width.min(ui.available_width());
        egui_extras::StripBuilder::new(ui)
            .size(egui_extras::Size::exact(width))
            .vertical(|mut strip| {
                strip.strip(|builder| {
                    builder
                        .size(egui_extras::Size::remainder())
                        .size(egui_extras::Size::exact(width))
                        .size(egui_extras::Size::remainder())
                        .horizontal(|mut strip| {
                            strip.empty();
                            strip.cell(|ui| {
                                let rect = ui.available_rect_before_wrap();
                                if let Some(texture_id) = texture_id {
                                    let uv = egui::Rect::from_min_max(
                                        egui::Pos2::ZERO,
                                        egui::Pos2::new(1.0, 1.0),
                                    );
                                    ui.painter()
                                        .image(texture_id, rect, uv, egui::Color32::WHITE);
                                } else {
                                    ui.painter().rect_filled(
                                        rect,
                                        0.0,
                                        egui::Color32::from_rgb(128, 128, 0),
                                    );
                                }
                            });
                            strip.empty();
                        });
                });
            });
    }
}
