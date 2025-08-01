use super::lines::RenderLines;
use super::mesh::RenderMesh;
use super::render_resource::RenderResourceComponent;
use super::render_resource::RenderResourceManager;
use crate::model::scene::LightComponent;
use crate::model::scene::Node;
use crate::model::scene::ShapeComponent;
use crate::render::render_mode::RenderMode;
use crate::render::scene_item::*;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::wgpu;

#[derive(Debug, Clone)]
pub struct MeshRenderItem {
    pub mesh: Arc<RenderMesh>,
    pub matrix: glam::Mat4,
}

#[derive(Debug, Clone)]
pub struct LinesRenderItem {
    pub lines: Arc<RenderLines>,
    pub matrix: glam::Mat4,
}

#[derive(Debug, Clone)]
pub enum RenderItem {
    Mesh(MeshRenderItem),
    Lines(LinesRenderItem),
    // Add other render item types here as needed
}

impl RenderItem {
    pub fn get_matrix(&self) -> glam::Mat4 {
        match self {
            RenderItem::Mesh(item) => item.matrix,
            RenderItem::Lines(item) => item.matrix,
            // Handle other render item types here
        }
    }
}

pub fn get_mesh(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    node: &Arc<RwLock<Node>>,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<Arc<RenderMesh>> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<ShapeComponent>() {
        let shape = component.get_shape();
        let shape = shape.read().unwrap();
        let mesh_id = shape.get_id();
        if let Some(mesh) = render_resource_manager.get_mesh(mesh_id) {
            if mesh.edition == shape.get_edition() {
                return Some(mesh.clone());
            }
        }
        if let Some(mesh) = RenderMesh::from_shape(device, queue, &shape) {
            let mesh = Arc::new(mesh);
            render_resource_manager.add_mesh(&mesh);
            return Some(mesh);
        }
    }
    return None;
}

fn get_light_gizmo(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    node: &Arc<RwLock<Node>>,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<Arc<RenderLines>> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        let light = component.get_light();
        let light = light.read().unwrap();
        let light_id = light.get_id();
        if let Some(lines) = render_resource_manager.get_lines(light_id) {
            if lines.edition == light.get_edition() {
                return Some(lines.clone());
            }
        }
        if let Some(lines) = RenderLines::from_light(device, queue, &light) {
            let lines = Arc::new(lines);
            render_resource_manager.add_lines(&lines);
            return Some(lines);
        }
    }
    return None;
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
    let resource_manager = get_render_resource_manager(node);
    let mut resource_manager = resource_manager.write().unwrap();
    for item in scene_items {
        match item.category {
            SceneItemType::Mesh => {
                if let Some(mesh) = get_mesh(device, queue, &item.node, &mut resource_manager) {
                    let matrix = glam::Mat4::from(item.matrix);
                    let render_item = MeshRenderItem { mesh, matrix };
                    render_items.push(Arc::new(RenderItem::Mesh(render_item)));
                }
            }
            SceneItemType::Light => {
                if let Some(lines) =
                    get_light_gizmo(device, queue, &item.node, &mut resource_manager)
                {
                    let matrix = glam::Mat4::from(item.matrix);
                    let render_item = LinesRenderItem { lines, matrix };
                    render_items.push(Arc::new(RenderItem::Lines(render_item)));
                }
            }
            // Handle other categories like Light, Camera, etc.
            _ => {}
        }
    }
    return render_items;
}
