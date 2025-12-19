use super::dyna_image::DynaImage;

use super::render_texture_image::render_texture_image;
use super::texture_node::TextureNode;
use super::texture_node::TextureSizeType;
use crate::model::scene::ResourceCacheManager;
use crate::model::scene::ResourceManager;

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::RwLock;

fn is_no_variant(node: &TextureNode, size_type: TextureSizeType) -> bool {
    return node.image_variants.get(&size_type).is_none();
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
            //let current_name = current_node.read().unwrap().name.clone();
            let current_id = current_node.read().unwrap().id;
            if !visited.contains(&current_id) {
                let dependencies = current_node.read().unwrap().inputs.clone();
                let dependencies = dependencies
                    .iter()
                    .filter_map(|(_key, dep)| {
                        if let Some(dep) = dep {
                            dep.upgrade()
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>();
                let dependencies = dependencies
                    .iter()
                    .filter(|dep| !visited.contains(&dep.read().unwrap().id))
                    .cloned()
                    .collect::<Vec<_>>();
                if !dependencies.is_empty() {
                    stack.push(current_node.clone());
                    for dep_node in dependencies.iter() {
                        stack.push(dep_node.clone());
                    }
                    continue;
                } else {
                    visited.insert(current_id);
                    ordered_nodes.push(current_node.clone());
                }
            }
        }
    }
    return ordered_nodes;
}

fn create_image_variants_for_nodes(
    texture_nodes: &Vec<Arc<RwLock<TextureNode>>>,
    resource_manager: &ResourceManager,
    size_type: TextureSizeType,
) {
    let ordered_nodes = sort_texture_nodes_by_dependency(&texture_nodes);
    /*
    println!("Creating image variants for size_type: {:?}", size_type);
    for (i, node) in texture_nodes.iter().enumerate() {
        let node = node.read().unwrap();
        let id = node.id;
        let name = node.name.clone();
        let ty = node.ty.clone();
        println!("{}: {} : {} ({})", i, ty, name, id);
    }
    println!("Ordered nodes:");
    for (i, node) in ordered_nodes.iter().enumerate() {
        let node = node.read().unwrap();
        let id = node.id;
        let name = node.name.clone();
        let ty = node.ty.clone();
        println!("{}: {} : {} ({})", i, ty, name, id);
    }
    */
    for texture_node in ordered_nodes.iter() {
        let mut texture_node = texture_node.write().unwrap();
        let texture_id = texture_node.id;
        let mut dependencies = HashMap::new();
        for (key, dep) in texture_node.inputs.iter() {
            if let Some(dep_node) = dep.as_ref() {
                let dep_node = dep_node.upgrade().unwrap();
                let dep_node = dep_node.read().unwrap();
                if let Some(image) = dep_node.image_variants.get(&size_type) {
                    dependencies.insert(key.clone(), image.clone());
                } else {
                    // should not happen
                    //println!(
                    //    "Warning: Dependency image variant not found for key: {} in texture node: {}",
                    //    key, dep_node.name
                    //);
                }
            }
        }
        let texture = resource_manager.textures.get(&texture_id).unwrap();
        let texture = texture.read().unwrap();
        if let Some(image) = render_texture_image(&texture, &dependencies, size_type) {
            texture_node
                .image_variants
                .insert(size_type, Arc::new(RwLock::new(image)));
        }
    }
}

pub fn create_image_variant(
    texture_node: &Arc<RwLock<TextureNode>>,
    resource_manager: &ResourceManager,
    size_type: TextureSizeType,
) -> Option<Arc<RwLock<DynaImage>>> {
    if let Some(image) = texture_node.read().unwrap().image_variants.get(&size_type) {
        return Some(image.clone());
    } else {
        let texture_nodes = vec![texture_node.clone()];
        create_image_variants_for_nodes(&texture_nodes, resource_manager, size_type);
        if let Some(image) = texture_node.read().unwrap().image_variants.get(&size_type) {
            return Some(image.clone());
        }
        return None;
    }
}

pub fn create_image_variants(
    resource_manager: &ResourceManager,
    resource_cache_manager: &mut ResourceCacheManager,
    size_type: TextureSizeType,
) {
    let mut texture_nodes = Vec::new();
    for (_id, texture_node) in resource_cache_manager.textures.iter() {
        if is_no_variant(&texture_node.read().unwrap(), size_type) {
            texture_nodes.push(texture_node.clone());
        }
    }
    create_image_variants_for_nodes(&texture_nodes, resource_manager, size_type);
}
