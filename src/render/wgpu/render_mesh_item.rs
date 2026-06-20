use super::mesh::RenderMesh;
use super::render_material_cache::get_render_material;
use super::render_item::MeshRenderItem;
use super::render_item::RenderItem;
use super::render_resource::RenderResourceManager;
use crate::model::scene::Node;
use crate::model::scene::RenderMeshComponent;
use crate::model::scene::ResourceManager;
use crate::model::scene::ShapeComponent;
use crate::render::render_mode::RenderMode;
use crate::render::scene_item::*;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::wgpu;

pub fn get_mesh(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    node: &Arc<RwLock<Node>>,
) -> Option<Arc<RenderMesh>> {
    let mut node = node.write().unwrap();
    let shape_component = node.get_component::<ShapeComponent>()?.clone();
    let shape = shape_component.get_shape();
    let shape = shape.read().unwrap();

    if let Some(component) = node.get_component::<RenderMeshComponent>()
        && let Some(render_mesh) = component.get_render_mesh()
        && render_mesh.edition == shape.get_edition()
    {
        return Some(render_mesh);
    }

    let mesh = RenderMesh::from_shape(device, queue, &shape).map(Arc::new)?;
    if let Some(component) = node.get_component_mut::<RenderMeshComponent>() {
        component.set_render_mesh(mesh.clone());
    } else {
        let mut component = RenderMeshComponent::new();
        component.set_render_mesh(mesh.clone());
        node.add_component(component);
    }
    Some(mesh)
}

pub fn get_render_mesh_item(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    item: &SceneItem,
    mode: RenderMode,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    if let Some(mesh) = get_mesh(device, queue, &item.node) {
        let matrix = glam::Mat4::from(item.matrix);
        let material = if mode == RenderMode::Lighting {
            get_render_material(
                device,
                queue,
                &item.node,
                resource_manager,
                render_resource_manager,
            )
        } else {
            None
        };
        let render_item = MeshRenderItem {
            mesh,
            material,
            matrix,
        };
        return Some(RenderItem::Mesh(render_item));
    }
    return None;
}
