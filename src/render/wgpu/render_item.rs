use super::mesh::RenderMesh;
use super::render_resource::RenderResourceComponent;
use super::render_resource::RenderResourceManager;
use crate::model::base::Matrix4x4;
use crate::model::scene::Node;
use crate::model::scene::ShapeComponent;
use crate::render::render_mode::RenderMode;
use crate::render::scene_item::*;
use crate::render::wgpu::render_resource;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::wgpu;
use eframe::wgpu::wgc::device::queue;

#[derive(Debug, Clone)]
pub struct MeshRenderItem {
    pub mesh: Arc<RenderMesh>,
    pub matrix: glam::Mat4,
    //pub material: Option<Arc<RenderMesh>>,
}

#[derive(Debug, Clone)]
pub struct LineGizmoItem {
    //todo
    pub matrix: glam::Mat4,
}

pub enum RenderItem {
    Mesh(MeshRenderItem),
    LineGizmo(LineGizmoItem),
    // Add other render item types here as needed
}

pub fn convert_matrix(m: &Matrix4x4) -> glam::Mat4 {
    return glam::Mat4::from_cols_array(&m.m);
}

pub fn get_mesh(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    node: &Arc<RwLock<Node>>,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<(Arc<RenderMesh>)> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<ShapeComponent>() {
        let shape = component.get_shape();
        let shape = shape.read().unwrap();
        let mesh_id = shape.get_id();
        if let Some(mesh) = render_resource_manager.get_mesh(mesh_id) {
            return Some(mesh.clone());
        } else {
            if let Some(mesh) = RenderMesh::from_shape(device,  queue, &shape) {
                let mesh = Arc::new(mesh);
                render_resource_manager.add_mesh(&mesh);
                return Some(mesh);
            }
        }
    }
    None
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
                    let matrix = convert_matrix(&item.matrix);
                    let render_item = MeshRenderItem { mesh, matrix };
                    render_items.push(Arc::new(RenderItem::Mesh(render_item)));
                }
            }
            // Handle other categories like Light, Camera, etc.
            _ => {}
        }
    }
    return render_items;
}
