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
                inputs: HashMap::new(),
                outputs: HashMap::new(),
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

        {
            let mut texture_node = texture_node.write().unwrap();
            for (_id, output) in texture_node.outputs.iter() {
                if let Some(output) = output.upgrade() {
                    let mut output = output.write().unwrap();
                    output.image_variants.clear(); // clear image variants of dependent nodes
                }
            }
            for (_key, input) in texture_node.inputs.iter() {
                if let Some(input) = input {
                    if let Some(input) = input.upgrade() {
                        let mut input = input.write().unwrap();
                        input.outputs.remove(&id); // remove this node from outputs of input nodes
                    }
                }
            }
            texture_node.inputs.clear(); // clear existing dependencies
            //texture_node.outputs.clear(); // clear existing dependencies
            texture_node.image_variants.clear(); // clear existing image variants
        }

        let mut dependency_nodes = Vec::new();
        {
            let mut texture_node = texture_node.write().unwrap();
            texture_node.properties = texture.as_property_map().clone();
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
                                if let Some(dep_texture_node) =
                                    resource_cache_manager.textures.get(&dep_id)
                                {
                                    dependency_nodes.push((key.clone(), dep_texture_node.clone()));
                                }
                            }
                        }
                    }
                }
            }
            texture_node.edition = edition; // update edition
        }
        {
            for (key, dep_texture_node) in dependency_nodes.iter() {
                TextureNode::set_link(key, &dep_texture_node, &texture_node);
            }
        }
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
