use super::super::panel::InspectorPanel;
use crate::conversion::texture_cache;
//use super::super::common::*;
use crate::conversion::texture_cache::TextureCacheManager;
use crate::conversion::texture_cache::TextureCacheSize;
use crate::model::scene::Node;
use crate::model::scene::ResourceCacheComponent;
use crate::model::scene::Texture;

use std::sync::{Arc, RwLock};

use eframe::egui;
use image::DynamicImage;

fn get_image_data(image: &DynamicImage) -> Option<egui::ColorImage> {
    let rgb_image = image.to_rgb8();
    let size = [rgb_image.width() as usize, rgb_image.height() as usize];
    let pixels = rgb_image.into_raw();
    Some(egui::ColorImage::from_rgb(size, &pixels))
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
    pub fn show_texture_preview(&self, ui: &mut egui::Ui, width: f32, texture: &Texture) {
        let name = format!("{}_{}", texture.get_name(), TextureCacheSize::Full);
        let controller = self.app_controller.read().unwrap();
        let root_node = controller.get_root_node();
        let mut texture_id = None;
        if let Some(texture_cache_manager) = get_texture_cache_manager(&root_node) {
            let texture_cache_manager = texture_cache_manager.read().unwrap();
            if let Some(texture_cache) =
                texture_cache_manager.get_texture_cache(texture, TextureCacheSize::Full)
            {
                let texture_cache = texture_cache.read().unwrap();
                let org_id = texture_cache.id;
                let mut texture_id_map = self.texture_id_map.write().unwrap();
                if let Some((edition, tex_id)) = texture_id_map.get(&org_id) {
                    if *edition == texture_cache.edition {
                        texture_id = Some(*tex_id);
                    }
                }
                if texture_id.is_none() {
                    // If the texture ID is not found in the map, create a new one
                    if let Some(rgb_image) = get_image_data(&texture_cache.image) {
                        let texture_manager = ui.ctx().tex_manager();
                        let mut texture_manager = texture_manager.write();

                        if let Some(old_tex_id) = texture_id_map.get(&org_id).map(|(_, id)| *id) {
                            texture_manager.free(old_tex_id);
                        }

                        // Create a new texture ID
                        let color_image = egui::ImageData::Color(Arc::new(rgb_image));
                        let texture_options = egui::TextureOptions::LINEAR;
                        let new_tex_id =
                            texture_manager.alloc(name.clone(), color_image, texture_options);
                        texture_id = Some(new_tex_id);
                        // Store the texture ID in the map
                        texture_id_map.insert(org_id, (texture_cache.edition.clone(), new_tex_id));
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
