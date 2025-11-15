use super::light::RenderLight;
use super::lines::RenderLines;
use super::material::RenderCategory;
use super::material::RenderMaterial;
use super::material::RenderPass;
use super::material::RenderUniformValue;
use super::mesh::RenderMesh;
use super::render_gizmo_item::get_render_axis_gizmo_items;
use super::render_gizmo_item::get_render_grid_gizmo_items;
use super::render_light_item::get_render_light_gizmo_item;
use super::render_light_item::get_render_light_items;
use super::render_mesh_item::get_render_mesh_item;
use super::render_resource::RenderResourceComponent;
use super::render_resource::RenderResourceManager;
use super::texture::RenderTexture;
use crate::conversion::spectrum::Spectrum;
use crate::conversion::texture_node::create_texture_nodes;
use crate::model::base::Property;
use crate::model::base::PropertyMap;
use crate::model::scene::Node;
use crate::model::scene::ResourceCacheComponent;
use crate::model::scene::ResourceCacheManager;
use crate::model::scene::ResourceComponent;
use crate::model::scene::ResourceManager;
use crate::render::render_mode::RenderMode;
use crate::render::scene_item::*;
use crate::render::wgpu::texture;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::wgpu;

#[derive(Debug, Clone)]
pub struct MeshRenderItem {
    pub mesh: Arc<RenderMesh>,
    pub material: Option<Arc<RenderMaterial>>,
    pub matrix: glam::Mat4,
}

#[derive(Debug, Clone)]
pub struct LinesRenderItem {
    pub lines: Arc<RenderLines>,
    pub material: Option<Arc<RenderMaterial>>,
    pub matrix: glam::Mat4,
}

#[derive(Debug, Clone)]
pub struct RenderLightItem {
    pub light: Arc<RenderLight>,
    pub matrix: glam::Mat4,
}

#[derive(Debug, Clone)]
pub enum RenderItem {
    Mesh(MeshRenderItem),
    Light(RenderLightItem),
    Lines(LinesRenderItem),
    // Add other render item types here as needed
}

impl RenderItem {
    pub fn get_matrix(&self) -> glam::Mat4 {
        match self {
            RenderItem::Mesh(item) => item.matrix,
            RenderItem::Light(item) => item.matrix,
            RenderItem::Lines(item) => item.matrix,
            // Handle other render item types here
        }
    }
    pub fn get_material(&self) -> Option<Arc<RenderMaterial>> {
        match self {
            RenderItem::Mesh(item) => item.material.clone(),
            RenderItem::Lines(item) => item.material.clone(),
            _ => None,
        }
    }
}

pub fn get_bool(props: &PropertyMap, key: &str) -> Option<bool> {
    if let Some((_key_type, _key_name, value)) = props.entry(key) {
        if let Property::Bools(v) = value {
            if v.len() >= 1 {
                return Some(v[0]);
            }
        }
    }
    return None;
}

pub fn get_float(props: &PropertyMap, key: &str) -> Option<f32> {
    if let Some((_key_type, _key_name, value)) = props.entry(key) {
        if let Property::Floats(v) = value {
            if v.len() >= 1 {
                return Some(v[0]);
            }
        }
    }
    return None;
}

pub fn get_color(
    props: &PropertyMap,
    key: &str,
    resource_manager: &ResourceManager,
) -> Option<[f32; 4]> {
    if let Some((key_type, _key_name, value)) = props.entry(key) {
        if key_type == "blackbody" {
            if let Property::Floats(v) = value {
                if v.len() >= 2 {
                    let s = Spectrum::from_blackbody(&v);
                    let rgb = s.to_rgb();
                    return Some([rgb[0], rgb[1], rgb[2], 1.0]);
                }
            }
        } else if key_type == "spectrum" {
            if let Property::Strings(v) = value {
                if v.len() >= 1 {
                    let name = v[0].clone();
                    if let Some(resource) = resource_manager.find_spectrum_by_filename(&name) {
                        let resource = resource.read().unwrap();
                        if let Some(fullpath) = resource.get_fullpath() {
                            if let Ok(spectrum) = Spectrum::load_from_file(&fullpath) {
                                let rgb = spectrum.to_rgb();
                                return Some([rgb[0], rgb[1], rgb[2], 1.0]);
                            }
                        }
                    }
                    log::warn!(
                        "Spectrum resource not found for key: {} with value: {}",
                        key,
                        name
                    );
                }
            }
        } else if key_type == "float" {
            if let Property::Floats(v) = value {
                if v.len() >= 1 {
                    // Assuming the first three values are RGB
                    return Some([v[0], v[0], v[0], 1.0]);
                }
            }
        } else {
            if let Property::Floats(v) = value {
                if v.len() >= 3 {
                    return Some([v[0], v[1], v[2], 1.0]);
                }
            }
        }
    }
    return None;
}

pub fn get_texture(
    props: &PropertyMap,
    key: &str,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<Arc<RenderTexture>> {
    if let Some((key_type, _key_name, value)) = props.entry(key) {
        if key_type == "texture" {
            if let Property::Strings(v) = value {
                if v.len() >= 1 {
                    let name = v[0].clone();
                    if let Some(texture) = resource_manager.find_texture_by_filename(&name) {
                        let texture = texture.read().unwrap();
                        let texture_id = texture.get_id();
                        let texture_edition = texture.get_edition();
                        if let Some(texture) = render_resource_manager.get_texture(texture_id) {
                            if texture.edition == texture_edition {
                                return Some(texture.clone());
                            }
                        }
                    }
                }
            }
        }
    }
    return None;
}

pub fn get_shader_type(
    shader_type: &str,
    uniform_values: &Vec<(String, RenderUniformValue)>,
) -> String {
    let mut s = "".to_string();
    for (key, val) in uniform_values.iter() {
        match val {
            RenderUniformValue::Float(_) => {
                s.push_str(&format!("_{}@F", key));
            }
            RenderUniformValue::Vec4(_) => {
                s.push_str(&format!("_{}@V", key));
            }
            RenderUniformValue::Int(_) => {
                s.push_str(&format!("_{}@I", key));
            }
            RenderUniformValue::Bool(_) => {
                s.push_str(&format!("_{}@B", key));
            }
            RenderUniformValue::Mat4(_) => {
                s.push_str(&format!("_{}@M", key));
            }
            RenderUniformValue::Texture(_) => {
                s.push_str(&format!("_{}@T", key));
            }
        }
    }
    return format!("{}{}", shader_type, s);
}

fn create_uniform_value_bytes(uniform_values: &[(String, RenderUniformValue)]) -> Vec<u8> {
    todo!()
}

pub fn create_render_pass(
    shader_type: &str,
    render_category: RenderCategory,
    uniform_values: &[(String, RenderUniformValue)],
    _render_resource_manager: &mut RenderResourceManager,
) -> RenderPass {
    let uniform_values_bytes = create_uniform_value_bytes(uniform_values);
    let render_pass = RenderPass {
        shader_type: shader_type.to_string(),
        render_category,
        uniform_values: uniform_values_bytes,
    };
    return render_pass;
}

fn get_resource_manager(node: &Arc<RwLock<Node>>) -> Arc<RwLock<ResourceManager>> {
    let mut node = node.write().unwrap();
    if node.get_component::<ResourceComponent>().is_none() {
        node.add_component::<ResourceComponent>(ResourceComponent::new());
    }
    let component = node.get_component::<ResourceComponent>().unwrap();
    return component.get_resource_manager();
}

fn get_render_resource_manager(node: &Arc<RwLock<Node>>) -> Arc<RwLock<RenderResourceManager>> {
    let mut node = node.write().unwrap();
    if node.get_component::<RenderResourceComponent>().is_none() {
        node.add_component::<RenderResourceComponent>(RenderResourceComponent::new());
    }
    let component = node.get_component::<RenderResourceComponent>().unwrap();
    return component.get_resource_manager();
}

fn get_resource_cache_manager(node: &Arc<RwLock<Node>>) -> Arc<RwLock<ResourceCacheManager>> {
    let mut node = node.write().unwrap();
    if node.get_component::<ResourceCacheComponent>().is_none() {
        node.add_component::<ResourceCacheComponent>(ResourceCacheComponent::new());
    }
    let component = node.get_component::<ResourceCacheComponent>().unwrap();
    return component.get_resource_cache_manager();
}

pub fn get_render_items(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    node: &Arc<RwLock<Node>>,
    mode: RenderMode,
) -> Vec<Arc<RenderItem>> {
    let scene_items = get_scene_items(node);
    let mut render_items = Vec::new();
    let resource_manager = get_resource_manager(node);
    let resource_manager = resource_manager.read().unwrap();
    let resource_cache_manager = get_resource_cache_manager(node);
    let mut resource_cache_manager = resource_cache_manager.write().unwrap();
    let render_resource_manager = get_render_resource_manager(node);
    let mut render_resource_manager = render_resource_manager.write().unwrap();

    //
    create_texture_nodes(&resource_manager, &mut resource_cache_manager);
    //register_ltc_textures(device, queue, &mut render_resource_manager);
    //

    for item in scene_items.iter() {
        match item.category {
            SceneItemType::Mesh => {
                if let Some(render_item) = get_render_mesh_item(
                    device,
                    queue,
                    item,
                    mode,
                    &resource_manager,
                    &mut render_resource_manager,
                ) {
                    render_items.push(Arc::new(render_item));
                }
            }
            SceneItemType::Light => {
                if mode == RenderMode::Lighting {
                    let items = get_render_light_items(
                        device,
                        queue,
                        item,
                        mode,
                        &resource_manager,
                        &mut resource_cache_manager,
                        &mut render_resource_manager,
                    );
                    if !items.is_empty() {
                        render_items.extend(items);
                    }
                }
                if let Some(render_item) = get_render_light_gizmo_item(
                    device,
                    queue,
                    item,
                    mode,
                    &resource_manager,
                    &mut render_resource_manager,
                ) {
                    render_items.push(Arc::new(render_item));
                }
            }
            // Handle other categories like Light, Camera, etc.
            _ => {}
        }
    }
    //additional render items based on the mode
    {
        let display_world_axes = true; // This should be a setting or parameter
        if display_world_axes {
            render_items.extend(get_render_axis_gizmo_items(
                device,
                queue,
                node,
                mode,
                &resource_manager,
                &mut render_resource_manager,
            ));
        }
        let display_grid = true; // This should be a setting or parameter
        if display_grid {
            render_items.extend(get_render_grid_gizmo_items(
                device,
                queue,
                node,
                mode,
                &resource_manager,
                &mut render_resource_manager,
            ));
        }
    }

    return render_items;
}
