use super::render_item::{GizmoRenderItem, MeshRenderItem, RenderItem};
use super::render_mode::RenderMode;
use super::wireframe_program::create_wireframe_program;
use crate::models::base::Matrix4x4;
use crate::models::scene::Mesh;
use crate::models::scene::Node;
use crate::models::scene::{CameraComponent, LightComponent, Material};

use crate::models::scene::Component;
use crate::models::scene::MaterialComponent;
use crate::models::scene::MeshComponent;

use crate::models::scene::ShapeComponent;
use crate::models::scene::SubdivComponent;
use crate::models::scene::TransformComponent;

use crate::renderers::gl::GLResourceComponent;
use crate::renderers::gl::GLResourceManager;
use crate::renderers::gl::RenderMaterial;
use crate::renderers::gl::RenderMesh;
use crate::renderers::gl::RenderProgram;

use eframe::glow;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SceneItemType {
    Mesh,
    Light,
    Camera,
}

struct SceneItem {
    pub node: Arc<RwLock<Node>>,
    pub category: SceneItemType, //type of the item (Mesh, Light, etc.)
    pub matrix: Matrix4x4,       //world matrix of the item
}

impl SceneItem {
    pub fn new(node: Arc<RwLock<Node>>, category: SceneItemType, matrix: Matrix4x4) -> Self {
        SceneItem {
            node,
            category,
            matrix,
        }
    }
}

fn has_component<T: Component>(node: &Arc<RwLock<Node>>) -> bool {
    let node = node.read().unwrap();
    node.get_component::<T>().is_some()
}

fn get_local_matrix(node: &Arc<RwLock<Node>>) -> Matrix4x4 {
    let node = node.read().unwrap();
    let t = node.get_component::<TransformComponent>().unwrap();
    return t.get_local_matrix();
}

fn get_material(node: &Arc<RwLock<Node>>) -> Arc<RwLock<Material>> {
    let node = node.read().unwrap();
    let m = node.get_component::<MaterialComponent>().unwrap();
    return m.material.clone();
}

fn get_scene_item(parent_matrix: &Matrix4x4, node: &Arc<RwLock<Node>>, items: &mut Vec<SceneItem>) {
    let local_matrix = get_local_matrix(node);
    let world_matrix = *parent_matrix * local_matrix;

    if has_component::<MeshComponent>(&node) && has_component::<MaterialComponent>(&node) {
        let item = SceneItem::new(node.clone(), SceneItemType::Mesh, world_matrix);
        items.push(item);
    }

    if has_component::<SubdivComponent>(&node) && has_component::<MaterialComponent>(&node) {
        let item = SceneItem::new(node.clone(), SceneItemType::Mesh, world_matrix);
        items.push(item);
    }

    if has_component::<ShapeComponent>(&node) && has_component::<MaterialComponent>(&node) {
        let item = SceneItem::new(node.clone(), SceneItemType::Mesh, world_matrix);
        items.push(item);
    }

    if has_component::<LightComponent>(&node) {
        let item = SceneItem::new(node.clone(), SceneItemType::Light, world_matrix);
        items.push(item);
    }

    if has_component::<CameraComponent>(&node) {
        let item = SceneItem::new(node.clone(), SceneItemType::Camera, world_matrix);
        items.push(item);
    }

    let node = node.read().unwrap();
    for child in &node.children {
        get_scene_item(&world_matrix, child, items);
    }
}

fn get_scene_items(node: &Arc<RwLock<Node>>) -> Vec<SceneItem> {
    let mut items = Vec::new();
    let parent_matrix = Matrix4x4::identity();
    get_scene_item(&parent_matrix, node, &mut items);
    items
}

fn convert_mesh(
    resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    mesh: &Mesh,
) -> Option<Arc<RenderMesh>> {
    let id = mesh.get_id();
    if let Some(render_mesh) = resource_manager.get_mesh(id) {
        return Some(render_mesh.clone());
    } else {
        if let Some(render_mesh) = RenderMesh::from_mesh(gl, mesh) {
            let render_mesh = Arc::new(render_mesh);
            resource_manager.add_mesh(&render_mesh);
            return Some(render_mesh);
        }
    }
    return None;
}

fn get_render_mesh(
    resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    node: &Arc<RwLock<Node>>,
) -> Option<Arc<RenderMesh>> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<MeshComponent>() {
        if let Some(mesh) = component.mesh.as_ref() {
            let mesh = mesh.read().unwrap();
            return convert_mesh(resource_manager, &gl, &mesh);
        }
    } else if let Some(component) = node.get_component::<SubdivComponent>() {
        if let Some(mesh) = component.mesh.as_ref() {
            let mesh = mesh.read().unwrap();
            return convert_mesh(resource_manager, &gl, &mesh);
        }
    } else if let Some(component) = node.get_component::<ShapeComponent>() {
        if let Some(mesh) = component.mesh.as_ref() {
            let mesh = mesh.read().unwrap();
            let rm = convert_mesh(resource_manager, &gl, &mesh);
            if let Some(rm) = rm.as_ref() {
                if rm.edition
                    != mesh
                        .as_property_map()
                        .find_one_string("edition")
                        .unwrap_or("".to_string())
                {
                    if let Some(new_mesh) = RenderMesh::from_mesh(&gl, &mesh) {
                        let new_mesh = Arc::new(new_mesh);
                        resource_manager.add_mesh(&new_mesh);
                        return Some(new_mesh.clone());
                    }
                }
            }
        }
    }
    return None;
}

fn convert_shader(
    resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    material: &Arc<RwLock<Material>>,
    _mode: RenderMode,
) -> Option<Arc<RenderProgram>> {
    let id = material.read().unwrap().get_id(); //
    if let Some(render_program) = resource_manager.get_program(id) {
        return Some(render_program.clone());
    } else {
        if let Some(render_program) = create_wireframe_program(gl, id) {
            resource_manager.add_program(&render_program);
            return Some(render_program);
        }
    }
    return None;
}

fn convert_material(
    resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    material: &Arc<RwLock<Material>>,
    mode: RenderMode,
) -> Option<Arc<RenderMaterial>> {
    let id = material.read().unwrap().get_id();
    if let Some(program) = convert_shader(resource_manager, gl, material, mode) {
        let render_material = RenderMaterial { id, program };
        let render_material = Arc::new(render_material);
        resource_manager.add_material(&render_material);
        return Some(render_material);
    }
    return None;
}

fn get_render_material(
    resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    node: &Arc<RwLock<Node>>,
    mode: RenderMode,
) -> Option<Arc<RenderMaterial>> {
    let material = get_material(node);
    return convert_material(resource_manager, gl, &material, mode);
}

pub fn get_render_items(
    gl: &Arc<glow::Context>,
    root_node: &Arc<RwLock<Node>>,
    mode: RenderMode,
) -> Vec<RenderItem> {
    let scene_items = get_scene_items(root_node);
    let mut root_node = root_node.write().unwrap();
    if root_node
        .get_component_mut::<GLResourceComponent>()
        .is_none()
    {
        root_node.add_component::<GLResourceComponent>(GLResourceComponent::new(gl));
    }
    let mut render_items: Vec<RenderItem> = Vec::new();
    if let Some(component) = root_node.get_component::<GLResourceComponent>() {
        let resource_manager = component.get_resource_manager();
        let mut resource_manager = resource_manager.lock().unwrap();
        for scene_item in scene_items.iter() {
            let node = scene_item.node.clone();
            let category = scene_item.category;
            let local_to_world = scene_item.matrix;

            match category {
                SceneItemType::Mesh => {
                    if let Some(mesh) = get_render_mesh(&mut resource_manager, gl, &node) {
                        if let Some(material) =
                            get_render_material(&mut resource_manager, gl, &node, mode)
                        {
                            let render_item = MeshRenderItem {
                                local_to_world,
                                mesh,
                                material,
                            };
                            render_items.push(RenderItem::Mesh(render_item));
                        }
                    }
                }
                SceneItemType::Light => {
                    //
                }
                SceneItemType::Camera => {}
                _ => {
                    // Handle other types if needed
                }
            }
        }
    }
    return render_items;
}
