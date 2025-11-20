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
use crate::conversion::texture_node::DynaImage;
use crate::conversion::texture_node::TexturePurpose;
use crate::conversion::texture_node::create_image_variants;
use crate::conversion::texture_node::create_texture_nodes;
use crate::model::base::Property;
use crate::model::base::PropertyMap;
use crate::model::scene::Node;
use crate::model::scene::ResourceCacheComponent;
use crate::model::scene::ResourceCacheManager;
use crate::model::scene::ResourceComponent;
use crate::model::scene::ResourceManager;
use crate::render;
use crate::render::render_mode::RenderMode;
use crate::render::scene_item::*;
//use crate::render::wgpu::texture;

use std::sync::Arc;
use std::sync::RwLock;
use std::vec;

use eframe::glow::Texture;
use eframe::wgpu::wgc::device::resource;
use uuid::Uuid;
//use bytemuck::{Pod, Zeroable};

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
                    if let Some(texture) = resource_manager.find_texture_by_name(&name) {
                        let texture = texture.read().unwrap();
                        let texture_id = texture.get_id();
                        //let texture_edition = texture.get_edition();
                        if let Some(render_texture) =
                            render_resource_manager.get_texture(texture_id)
                        {
                            //println!("Get Render Texture from Cache: key={}, name={}", key, name);
                            //if render_texture.edition == texture_edition {
                            return Some(render_texture.clone());
                            //}
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

fn create_uniform_value_bytes(
    uniform_values: &[(String, RenderUniformValue)],
) -> (Vec<(String, String)>, Vec<u8>) {
    let mut type_variables: Vec<(String, String)> = Vec::new();
    let mut bytes: Vec<u8> = Vec::new();
    let mut padding_count = 1;
    let mut remain = 0;
    for (name, value) in uniform_values.iter() {
        assert!(remain >= 0);
        match value {
            RenderUniformValue::Float(v) => {
                bytes.extend_from_slice(bytemuck::bytes_of(v));
                if remain == 0 {
                    remain = 16; //std140 vec4 alignment
                }
                remain -= 4;
                type_variables.push(("f32".to_string(), name.clone())); //
            }
            RenderUniformValue::Vec4(v) => {
                if remain != 0 {
                    for _ in 0..remain {
                        bytes.extend_from_slice(bytemuck::bytes_of(&0.0f32));
                        type_variables.push(("f32".to_string(), format!("_pad{}", padding_count))); //
                        padding_count += 1;
                    }
                    remain = 0;
                }
                bytes.extend_from_slice(bytemuck::bytes_of(v));
                type_variables.push(("vec4<f32>".to_string(), name.clone())); //
            }
            RenderUniformValue::Int(v) => {
                bytes.extend_from_slice(bytemuck::bytes_of(v));
                if remain == 0 {
                    remain = 16; //std140 vec4 alignment
                }
                remain -= 4;
                type_variables.push(("i32".to_string(), name.clone())); //
            }
            RenderUniformValue::Bool(v) => {
                let int_value: u32 = if *v { 1 } else { 0 };
                bytes.extend_from_slice(bytemuck::bytes_of(&int_value));
                if remain == 0 {
                    remain = 16; //std140 vec4 alignment
                }
                remain -= 4;
                type_variables.push(("u32".to_string(), name.clone())); //
            }
            RenderUniformValue::Mat4(v) => {
                if remain != 0 {
                    for _ in 0..remain {
                        bytes.extend_from_slice(bytemuck::bytes_of(&0.0f32));
                        type_variables.push(("f32".to_string(), format!("_pad{}", padding_count))); //
                        padding_count += 1;
                    }
                    remain = 0;
                }
                bytes.extend_from_slice(bytemuck::bytes_of(v));
                type_variables.push(("mat4x4<f32>".to_string(), name.clone())); //
            }
            RenderUniformValue::Texture(_v) => {
                let scale_offset: [f32; 4] = [1.0, 1.0, 0.0, 0.0];
                bytes.extend_from_slice(bytemuck::bytes_of(&scale_offset));
                type_variables.push(("vec4<f32>".to_string(), format!("{}_uv_factor", name))); //
            }
        }
    }
    if remain != 0 {
        for _ in 0..remain {
            bytes.extend_from_slice(bytemuck::bytes_of(&0.0f32));
            type_variables.push(("f32".to_string(), format!("_pad{}", padding_count))); //
            padding_count += 1;
        }
        //remain = 0;
    }

    return (type_variables, bytes);
}

pub fn create_render_pass(
    shader_type: &str,
    render_category: RenderCategory,
    uniform_values: &[(String, RenderUniformValue)],
    _render_resource_manager: &mut RenderResourceManager,
) -> RenderPass {
    let (_uniform_values_types, uniform_values_bytes) = create_uniform_value_bytes(uniform_values);
    /*
    println!(
        "Create Render Pass: shader_type={}, uniform_values={:?}",
        shader_type, _uniform_values_types
    );
    */
    let mut textures = vec![];
    for (_name, value) in uniform_values.iter() {
        if let RenderUniformValue::Texture(texture) = value {
            textures.push(texture.clone());
        }
    }
    let render_pass = RenderPass {
        id: Uuid::new_v4(),
        shader_type: shader_type.to_string(),
        render_category,
        uniform_values: uniform_values_bytes,
        textures,
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

fn get_image_data(image: &DynaImage) -> image::Rgba32FImage {
    return image.to_rgba32f();
}

fn get_texture_from_image(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    image: &image::Rgba32FImage,
) -> wgpu::Texture {
    let dimensions = image.dimensions();
    let size = wgpu::Extent3d {
        width: dimensions.0,
        height: dimensions.1,
        depth_or_array_layers: 1,
    };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Render Texture"),
        size: size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let image_raw = image.as_raw();
    queue.write_texture(
        texture.as_image_copy(),
        bytemuck::cast_slice(image_raw),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * 4 * dimensions.0),
            rows_per_image: None,
        },
        size,
    );
    return texture;
}

fn create_render_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resource_manager: &ResourceManager,
    resource_cache_manager: &ResourceCacheManager,
    render_resource_manager: &mut RenderResourceManager,
    purpose: TexturePurpose,
) {
    for (_id, texture) in resource_manager.textures.iter() {
        let texture = texture.read().unwrap();
        let texture_id = texture.get_id();
        let texture_edition = texture.get_edition();
        if let Some(render_texture) = render_resource_manager.get_texture(texture_id) {
            if texture_edition == render_texture.edition {
                //println!("Skip Create Render Texture from Cache: id={}", id);
                continue;
            }
        }
        if let Some(texture_node) = resource_cache_manager.textures.get(&texture_id) {
            let texture_node = texture_node.read().unwrap();
            if let Some(image) = texture_node.image_variants.get(&purpose) {
                let image = image.read().unwrap();
                let image_data = get_image_data(&image);
                let texture = get_texture_from_image(device, queue, &image_data);
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("Render Texture Sampler"),
                    address_mode_u: wgpu::AddressMode::ClampToEdge,
                    address_mode_v: wgpu::AddressMode::ClampToEdge,
                    min_filter: wgpu::FilterMode::Linear,
                    mag_filter: wgpu::FilterMode::Linear,
                    ..Default::default()
                });

                let render_texture = RenderTexture {
                    id: texture_id,
                    edition: texture_edition.clone(),
                    texture,
                    view,
                    sampler,
                };
                let render_texture = Arc::new(render_texture);
                render_resource_manager.add_texture(&render_texture);
            }
        }
    }
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

    if mode == RenderMode::Lighting {
        create_texture_nodes(&resource_manager, &mut resource_cache_manager);
        create_image_variants(
            &resource_manager,
            &mut resource_cache_manager,
            TexturePurpose::Render,
        );
        create_render_textures(
            device,
            queue,
            &resource_manager,
            &resource_cache_manager,
            &mut render_resource_manager,
            TexturePurpose::Render,
        );
    }

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
