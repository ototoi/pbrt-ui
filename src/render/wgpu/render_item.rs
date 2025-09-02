use super::light::RenderLight;
use super::lines::RenderLines;
use super::material::RenderMaterial;
use super::mesh::RenderMesh;
use super::render_gizmo_item::get_render_axis_gizmo_items;
use super::render_gizmo_item::get_render_grid_gizmo_items;
use super::render_light_item::get_render_light_gizmo_item;
use super::render_light_item::get_render_light_items;
use super::render_mesh_item::get_render_mesh_item;
use super::render_resource::RenderResourceComponent;
use super::render_resource::RenderResourceManager;
use crate::conversion::spectrum::Spectrum;
use crate::model::base::Property;
use crate::model::base::PropertyMap;
use crate::model::scene::Node;
use crate::model::scene::ResourceComponent;
use crate::model::scene::ResourceManager;
use crate::render::render_mode::RenderMode;
use crate::render::scene_item::*;

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
    let render_resource_manager = get_render_resource_manager(node);
    let mut render_resource_manager = render_resource_manager.write().unwrap();
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
