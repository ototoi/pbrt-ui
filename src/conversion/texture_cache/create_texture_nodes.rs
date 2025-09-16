use super::texture_node::TextureDependent;
use super::texture_node::TextureNode;
use crate::model::base::Property;
use crate::model::scene::ResourceCacheManager;
use crate::model::scene::ResourceManager;
use crate::model::scene::Texture;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

fn initialize_texture_node(
    resource_manager: &ResourceManager,
    resource_cache_manager: &mut ResourceCacheManager,
) {
    for (_id, texture) in resource_manager.textures.iter() {
        let texture = texture.read().unwrap();
        let name = texture.get_name();
        let ty = texture.get_type();
        let id = texture.get_id();
        //let edition = texture.get_edition();
        if resource_cache_manager.textures.get(&id).is_none() {
            let texture_node = TextureNode {
                name,
                ty,
                id,
                properties: texture.as_property_map().clone(),
                edition: "".to_string(), // initially empty
                dependencies: HashMap::new(),
                image_variants: HashMap::new(),
            };
            let texture_node = Arc::new(RwLock::new(texture_node));
            resource_cache_manager
                .textures
                .insert(id.clone(), texture_node);
        }
    }
}

fn get_dependent_texture_keys(texture: &Texture) -> Vec<String> {
    let props = texture.as_property_map();
    let keys = props
        .get_keys()
        .iter()
        .filter(|(key_type, key_name)| {
            if let Some(value) = props.get(key_name) {
                match value {
                    Property::Strings(_) => key_type == "texture",
                    Property::Floats(_) => true,
                    _ => false,
                }
            } else {
                false
            }
        })
        .map(|(_key_type, key_name)| key_name.clone())
        .collect();
    return keys;
}

fn remove_stale_texture_nodes(
    resource_manager: &ResourceManager,
    resource_cache_manager: &mut ResourceCacheManager,
) {
    let mut stale_ids = Vec::new();
    for (id, _texture_node) in resource_cache_manager.textures.iter() {
        if resource_manager.textures.get(id).is_none() {
            stale_ids.push(*id);
        }
    }
    for id in stale_ids.iter() {
        resource_cache_manager.textures.remove(id);
    }
}

fn connect_texture_dependencies(
    resource_manager: &ResourceManager,
    resource_cache_manager: &mut ResourceCacheManager,
) {
    for (_id, texture) in resource_manager.textures.iter() {
        let texture = texture.read().unwrap();
        let id = texture.get_id();
        let edition = texture.get_edition();
        let texture_node = resource_cache_manager.textures.get(&id).unwrap(); // must exist
        {
            let texture_node = texture_node.read().unwrap();
            if texture_node.get_edition() == edition {
                // No update needed
                continue;
            }
        }
        let mut texture_node = texture_node.write().unwrap();
        texture_node.properties = texture.as_property_map().clone();
        texture_node.dependencies.clear(); // clear existing dependencies
        texture_node.image_variants.clear(); // clear existing image variants
        // connect dependencies
        let keys = get_dependent_texture_keys(&texture);
        let props = texture.as_property_map();
        for key in keys.iter() {
            if let Some(value) = props.get(key) {
                if let Property::Strings(names) = value {
                    for dep_texture_name in names {
                        if let Some(dep_texture) =
                            resource_manager.find_texture_by_name(dep_texture_name)
                        {
                            let dep_texture = dep_texture.read().unwrap();
                            let dep_id = dep_texture.get_id();
                            if let Some(dep_node) = resource_cache_manager.textures.get(&dep_id) {
                                texture_node
                                    .dependencies
                                    .insert(key.clone(), TextureDependent::Node(dep_node.clone()));
                            }
                        }
                    }
                } else if let Property::Floats(_value) = value {
                    //let value = Property::Floats(value.clone());
                    //texture_node
                    //.dependencies
                    //    .insert(key.clone(), TextureDependent::Value(value));
                }
            }
        }
        texture_node.edition = edition; // update edition
    }
}

pub fn create_texture_nodes(
    resource_manager: &ResourceManager,
    resource_cache_manager: &mut ResourceCacheManager,
) {
    initialize_texture_node(resource_manager, resource_cache_manager);
    remove_stale_texture_nodes(resource_manager, resource_cache_manager);
    connect_texture_dependencies(resource_manager, resource_cache_manager);
}
