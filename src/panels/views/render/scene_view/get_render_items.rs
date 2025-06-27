use super::gizmo_program::create_gizmo_program;
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

use crate::renderers::gl::GLResourceManager;
use crate::renderers::gl::RenderMaterial;
use crate::renderers::gl::RenderMesh;
use crate::renderers::gl::RenderProgram;
use crate::renderers::gl::{GLResourceComponent, RenderGizmo};

use uuid::Uuid;

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
    _mode: RenderMode,
) -> Option<Arc<RenderMesh>> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<MeshComponent>() {
        let mesh = component.mesh.clone();
        let mesh = mesh.read().unwrap();
        return convert_mesh(resource_manager, &gl, &mesh);
    } else if let Some(component) = node.get_component::<SubdivComponent>() {
        let mesh = component.mesh.clone();
        let mesh = mesh.read().unwrap();
        return convert_mesh(resource_manager, &gl, &mesh);
    } else if let Some(component) = node.get_component::<ShapeComponent>() {
        let mesh = component.mesh.clone();
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
    return None;
}

fn convert_shader_program(
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
    if let Some(program) = convert_shader_program(resource_manager, gl, material, mode) {
        let render_material = RenderMaterial {
            id,
            program,
            gl: gl.clone(),
        };
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

fn convert_light_gizmo(
    resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    component: &LightComponent,
) -> Option<Arc<RenderGizmo>> {
    let id = component.get_id();
    if let Some(gizmo) = resource_manager.get_gizmo(id) {
        return Some(gizmo.clone());
    } else {
        let light = component.light.read().unwrap();
        if let Some(gizmo) = RenderGizmo::from_light_shape(gl, &light) {
            let gizmo = Arc::new(gizmo);
            resource_manager.add_gizmo(&gizmo);
            return Some(gizmo);
        }
    }
    return None;
}

const GIZMO_SHADER_ID: &str = "a0eebc99-9c0b-4ef8-bb6d-6bb9bd380a11";
fn get_gizmo_shader_program(
    resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    gizmo: &Arc<RenderGizmo>,
    _mode: RenderMode,
) -> Option<Arc<RenderProgram>> {
    let id = Uuid::parse_str(GIZMO_SHADER_ID).unwrap();
    if let Some(program) = resource_manager.get_program(id) {
        return Some(program.clone());
    } else {
        if let Some(program) = create_gizmo_program(gl, id) {
            resource_manager.add_program(&program);
            return Some(program);
        }
    }
    None
}

fn get_light_render_gizmo(
    resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    component: &LightComponent,
    mode: RenderMode,
) -> Option<(Arc<RenderGizmo>, Arc<RenderMaterial>)> {
    if let Some(gizmo) = convert_light_gizmo(resource_manager, gl, component) {
        if let Some(program) = get_gizmo_shader_program(resource_manager, gl, &gizmo, mode) {
            //
            let render_material = RenderMaterial {
                id: gizmo.get_id(),
                program,
                gl: gl.clone(),
            };
            let render_material = Arc::new(render_material);
            resource_manager.add_material(&render_material);
            return Some((gizmo, render_material));
        }
    }
    None
}

fn get_camera_render_gizmo(
    resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    component: &CameraComponent,
    mode: RenderMode,
) -> Option<(Arc<RenderGizmo>, Arc<RenderMaterial>)> {
    None
}

fn get_render_gizmo(
    resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    node: &Arc<RwLock<Node>>,
    mode: RenderMode,
) -> Option<(Arc<RenderGizmo>, Arc<RenderMaterial>)> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        return get_light_render_gizmo(resource_manager, gl, component, mode);
    } else if let Some(component) = node.get_component::<CameraComponent>() {
        return get_camera_render_gizmo(resource_manager, gl, component, mode);
    }
    None
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
                    if let Some(mesh) = get_render_mesh(&mut resource_manager, gl, &node, mode) {
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
                    if let Some((gizmo, material)) =
                        get_render_gizmo(&mut resource_manager, gl, &node, mode)
                    {
                        let render_item = GizmoRenderItem {
                            local_to_world,
                            gizmo,
                            material,
                        };
                        render_items.push(RenderItem::Gizmo(render_item));
                    }
                }
                SceneItemType::Camera => {
                    if let Some((gizmo, material)) =
                        get_render_gizmo(&mut resource_manager, gl, &node, mode)
                    {
                        let render_item = GizmoRenderItem {
                            local_to_world,
                            gizmo,
                            material,
                        };
                        render_items.push(RenderItem::Gizmo(render_item));
                    }
                }
                _ => {
                    // Handle other types if needed
                }
            }
        }
    }
    return render_items;
}
