use image::DynamicImage;

use super::render_texture_image::render_texture_image;
use super::texture_node::TextureDependent;
use super::texture_node::TextureNode;
use super::texture_node::TexturePurpose;
use crate::model::scene::ResourceCacheManager;
use crate::model::scene::ResourceManager;

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::RwLock;

fn is_no_variant(node: &TextureNode, purpose: TexturePurpose) -> bool {
    return node.image_variants.get(&purpose).is_none();
}

fn sort_texture_nodes_by_dependency(
    nodes: &Vec<Arc<RwLock<TextureNode>>>,
) -> Vec<Arc<RwLock<TextureNode>>> {
    let mut ordered_nodes = Vec::new();
    let mut visited = HashSet::new();
    for node in nodes.iter() {
        let mut stack = Vec::new();
        stack.push(node.clone());
        while let Some(current_node) = stack.pop() {
            let current_id = current_node.read().unwrap().id;
            if !visited.contains(&current_id) {
                ordered_nodes.push(current_node.clone());
                visited.insert(current_id);
                let dependencies = current_node.read().unwrap().dependencies.clone();
                for (_key, dep) in dependencies.iter() {
                    if let TextureDependent::Node(dep_node) = dep {
                        let dep_id = dep_node.read().unwrap().id;
                        if !visited.contains(&dep_id) {
                            stack.push(dep_node.clone());
                        }
                    }
                }
            }
        }
    }
    ordered_nodes.reverse();
    return ordered_nodes;
}

fn linear_to_srgb(value: f32) -> u8 {
    if value <= 0.0031308 {
        (value * 12.92 * 255.0).round() as u8
    } else {
        ((1.055 * value.powf(1.0 / 2.4) - 0.055) * 255.0).round() as u8
    }
}

fn convert_float_to_u8(image: &image::Rgb32FImage) -> image::RgbImage {
    let (width, height) = image.dimensions();
    let mut u8_image = image::RgbImage::new(width, height);
    for (x, y, pixel) in image.enumerate_pixels() {
        let r = linear_to_srgb(pixel[0].clamp(0.0, 1.0));
        let g = linear_to_srgb(pixel[1].clamp(0.0, 1.0));
        let b = linear_to_srgb(pixel[2].clamp(0.0, 1.0));
        u8_image.put_pixel(x, y, image::Rgb([r, g, b]));
    }
    u8_image
}

fn convert_to_srgb_u8_image(image: &DynamicImage) -> image::RgbImage {
    match image {
        DynamicImage::ImageRgb8(img) => img.clone(),
        DynamicImage::ImageRgba8(_img) => {
            let rgb_image = image.to_rgb8();
            return rgb_image;
        }
        DynamicImage::ImageRgb32F(img) => {
            let rgb_image = convert_float_to_u8(img);
            return rgb_image;
        }
        DynamicImage::ImageRgba32F(_img) => {
            let img = image.to_rgb32f();
            let rgb_image = convert_float_to_u8(&img);
            return rgb_image;
        }
        DynamicImage::ImageLuma16(_img) => {
            let rgb_image = image.to_rgb8();
            return rgb_image;
        }
        _ => {
            let rgb_image = image.to_rgb8();
            return rgb_image;
        }
    }
}

fn create_image_variants_for_nodes(
    texture_nodes: &Vec<Arc<RwLock<TextureNode>>>,
    resource_manager: &ResourceManager,
    purpose: TexturePurpose,
) {
    let ordered_nodes = sort_texture_nodes_by_dependency(&texture_nodes);
    for texture_node in ordered_nodes.iter() {
        let mut texture_node = texture_node.write().unwrap();
        let texture_id = texture_node.id;
        let mut dependencies = HashMap::new();
        for (key, dep) in texture_node.dependencies.iter() {
            if let TextureDependent::Node(dep_node) = dep {
                let dep_node = dep_node.read().unwrap();
                if let Some(image) = dep_node.image_variants.get(&purpose) {
                    dependencies.insert(key.clone(), image.clone());
                }
            }
        }
        let texture = resource_manager.textures.get(&texture_id).unwrap();
        let texture = texture.read().unwrap();
        if let Some(image) = render_texture_image(&texture, &dependencies, purpose) {
            if purpose == TexturePurpose::Render {
                texture_node
                    .image_variants
                    .insert(purpose, Arc::new(RwLock::new(image)));
            } else {
                let srgb_purpose = purpose.add_srgb();
                let srgb_image = DynamicImage::ImageRgb8(convert_to_srgb_u8_image(&image));

                texture_node
                    .image_variants
                    .insert(purpose, Arc::new(RwLock::new(image)));

                texture_node
                    .image_variants
                    .insert(srgb_purpose, Arc::new(RwLock::new(srgb_image)));
            }
        }
    }
}

pub fn create_image_variant(
    texture_node: &Arc<RwLock<TextureNode>>,
    resource_manager: &ResourceManager,
    purpose: TexturePurpose,
) -> Option<Arc<RwLock<DynamicImage>>> {
    if let Some(image) = texture_node.read().unwrap().image_variants.get(&purpose) {
        return Some(image.clone());
    } else {
        let texture_nodes = vec![texture_node.clone()];
        create_image_variants_for_nodes(&texture_nodes, resource_manager, purpose);
        if let Some(image) = texture_node.read().unwrap().image_variants.get(&purpose) {
            return Some(image.clone());
        }
        return None;
    }
}

pub fn create_image_variants(
    resource_manager: &ResourceManager,
    resource_cache_manager: &mut ResourceCacheManager,
    purpose: TexturePurpose,
) {
    let mut texture_nodes = Vec::new();
    for (_id, texture_node) in resource_cache_manager.textures.iter() {
        if is_no_variant(&texture_node.read().unwrap(), purpose) {
            texture_nodes.push(texture_node.clone());
        }
    }
    create_image_variants_for_nodes(&texture_nodes, resource_manager, purpose);
}
