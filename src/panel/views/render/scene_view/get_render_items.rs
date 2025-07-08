use super::render_gizmo_program::{GIZMO_SHADER_ID, create_render_gizmo_program};
use super::render_item::{GizmoRenderItem, MeshRenderItem, RenderItem};
use super::render_mode::RenderMode;
use super::render_solid_program::{RENDER_SOLID_SHADER_COLOR_ID, RENDER_SOLID_SHADER_TEXTURE_ID, create_render_solid_program};
use super::render_wireframe_program::{WIREFRAME_SHADER_ID, create_render_wireframe_program};

use crate::model::base::Matrix4x4;
use crate::model::base::Property;
use crate::model::base::PropertyMap;
use crate::model::scene::Node;
use crate::model::scene::Shape;
use crate::model::scene::{CameraComponent, LightComponent, Material};

use crate::geometry::texture_cache::{self, TextureCacheManager, TextureCacheSize};
use crate::model::scene::Component;
use crate::model::scene::MaterialComponent;
use crate::model::scene::ResourceCacheComponent;
use crate::model::scene::ResourceComponent;
use crate::model::scene::ResourceManager;
use crate::model::scene::ShapeComponent;
use crate::model::scene::Texture;
use crate::model::scene::TransformComponent;

use crate::renderer::gl::GLResourceManager;
use crate::renderer::gl::RenderGizmo;
use crate::renderer::gl::RenderMaterial;
use crate::renderer::gl::RenderMesh;
use crate::renderer::gl::RenderProgram;
use crate::renderer::gl::RenderUniformValue;
use crate::renderer::gl::{GLResourceComponent, RenderTexture};

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
    render_resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    shape: &Shape,
) -> Option<Arc<RenderMesh>> {
    let id = shape.get_id();
    if let Some(rm) = render_resource_manager.get_mesh(id) {
        let shape_type = shape.get_type();
        if ShapeComponent::is_ediable(&shape_type) {
            if rm.get_edition() == shape.get_edition() {
                return Some(rm.clone());
            }
        } else {
            return Some(rm.clone());
        }
    }
    if let Some(render_mesh) = RenderMesh::from_mesh(gl, shape) {
        let render_mesh = Arc::new(render_mesh);
        render_resource_manager.add_mesh(&render_mesh);
        return Some(render_mesh);
    }

    return None;
}

fn get_render_mesh(
    render_resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    node: &Arc<RwLock<Node>>,
    _mode: RenderMode,
) -> Option<Arc<RenderMesh>> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<ShapeComponent>() {
        let shape = component.get_shape();
        let shape = shape.read().unwrap();
        if let Some(rm) = convert_mesh(render_resource_manager, &gl, &shape) {
            return Some(rm);
        }
    }
    return None;
}

fn get_shader_id(material: &Arc<RwLock<Material>>, uniforms: &[(String, RenderUniformValue)], mode: RenderMode) -> Uuid {
    match mode {
        RenderMode::Wireframe => {
            Uuid::parse_str(WIREFRAME_SHADER_ID).unwrap() // Placeholder for wireframe shader ID
        }
        RenderMode::Solid => {
            if let Some(base_color) = uniforms.iter().find(|(k, _)| k == "base_color") {
                if let RenderUniformValue::Vec4(color) = &base_color.1 {
                    // Use a unique ID based on the base color
                    return Uuid::parse_str(RENDER_SOLID_SHADER_COLOR_ID).unwrap() // Placeholder for solid shader ID
                } else if let RenderUniformValue::Texture(_) = &base_color.1 {
                    // Use a unique ID based on the texture
                    return Uuid::parse_str(RENDER_SOLID_SHADER_TEXTURE_ID).unwrap() // Placeholder for solid shader ID
                }
            }
            Uuid::parse_str(RENDER_SOLID_SHADER_COLOR_ID).unwrap() // Placeholder for solid shader ID
        }
        _ => {
            material.read().unwrap().get_id() // Use the material's ID for solid mode
        }
    }
}

fn convert_shader_program(
    render_resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    material: &Arc<RwLock<Material>>,
    uniforms: &[(String, RenderUniformValue)],
    mode: RenderMode,
) -> Option<Arc<RenderProgram>> {
    let id = get_shader_id(material, uniforms, mode);
    if let Some(render_program) = render_resource_manager.get_program(id) {
        return Some(render_program.clone());
    } else {
        match mode {
            RenderMode::Wireframe => {
                if let Some(render_program) = create_render_wireframe_program(gl, id) {
                    render_resource_manager.add_program(&render_program);
                    return Some(render_program);
                }
            }
            RenderMode::Solid => {
                if let Some(render_program) = create_render_solid_program(gl, id) {
                    render_resource_manager.add_program(&render_program);
                    return Some(render_program);
                }
            }
            _ => {
                // For other modes, use the material's ID to get the shader program
            }
        }
    }
    return None;
}

fn get_texture(
    render_resource_manager: &mut GLResourceManager,
    texture: &Arc<RwLock<Texture>>,
) -> Option<Arc<RenderTexture>> {
    let id = texture.read().unwrap().get_id();
    if let Some(render_texture) = render_resource_manager.get_texture(id) {
        return Some(render_texture.clone());
    }
    return None;
}

fn get_base_color_value(
    resource_manager: &ResourceManager,
    render_resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    props: &PropertyMap,
    key: &str,
) -> Option<RenderUniformValue> {
    if let Some((key_type, _key_name, value)) = props.entry(key) {
        if let Property::Floats(v) = value {
            if v.len() >= 3 {
                return Some(RenderUniformValue::Vec4([v[0], v[1], v[2], 1.0]));
            }
        } else if let Property::Strings(value) = value {
            if key_type == "texture" {
                let texture_name = value.get(0).cloned().unwrap_or_default();
                if let Some(texture) = resource_manager.find_texture_by_name(&texture_name) {
                    if let Some(render_texture) = get_texture(render_resource_manager, &texture) {
                        return Some(RenderUniformValue::Texture(render_texture.texture));
                    }
                }
                return None;
            } else if key_type == "spectrum" {
                //let spectrum
                return Some(RenderUniformValue::Vec4([1.0, 1.0, 1.0, 1.0]));
            }
            // Handle texture loading if needed
            return None;
        }
    }
    return None;
}

fn get_base_color(
    resource_manager: &ResourceManager,
    render_resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    material: &Arc<RwLock<Material>>,
) -> Option<RenderUniformValue> {
    let material = material.read().unwrap();
    let material_type = material.get_type();
    let props = material.as_property_map();
    match material_type.as_str() {
        "matte" | "plastic" | "translucent" | "uber" => {
            return get_base_color_value(
                resource_manager,
                render_resource_manager,
                gl,
                props,
                "Kd",
            );
        }
        "metal" => {
            return get_base_color_value(
                resource_manager,
                render_resource_manager,
                gl,
                props,
                "Kr",
            );
        }
        "glass" | "mirror" => {
            return get_base_color_value(
                resource_manager,
                render_resource_manager,
                gl,
                props,
                "Kr",
            );
        }
        "disney" => {
            return get_base_color_value(
                resource_manager,
                render_resource_manager,
                gl,
                props,
                "color",
            );
        }
        _ => {}
    }
    return None;
}

fn convert_material(
    resource_manager: &ResourceManager,
    render_resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    material: &Arc<RwLock<Material>>,
    mode: RenderMode,
) -> Option<Arc<RenderMaterial>> {
    let id = material.read().unwrap().get_id();
    let mut uniform_values = Vec::new();
    match mode {
        RenderMode::Wireframe => {
            uniform_values.push((
                "base_color".to_string(),
                RenderUniformValue::Vec4([1.0, 1.0, 1.0, 1.0]), //should replace with wireframe color
            ));
        }
        RenderMode::Solid => {
            if let Some(base_color) =
                get_base_color(resource_manager, render_resource_manager, gl, material)
            {
                uniform_values.push(("base_color".to_string(), base_color));
            } else {
                uniform_values.push((
                    "base_color".to_string(),
                    RenderUniformValue::Vec4([1.0, 1.0, 1.0, 1.0]), // Default white color
                ));
            }
        }
        _ => {
            //
        }
    }
    if let Some(program) = convert_shader_program(render_resource_manager, gl, material, &uniform_values, mode) {
        let edition = material.read().unwrap().get_edition();
        let render_material = RenderMaterial {
            id,
            edition,
            uniform_values,
            program,
            gl: gl.clone(),
        };
        let render_material = Arc::new(render_material);
        if mode != RenderMode::Wireframe {
            render_resource_manager.add_material(&render_material);
        }
        return Some(render_material);
    }
    return None;
}

fn get_render_material(
    resource_manager: &ResourceManager,
    render_resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    node: &Arc<RwLock<Node>>,
    mode: RenderMode,
) -> Option<Arc<RenderMaterial>> {
    let material = get_material(node);
    if mode != RenderMode::Wireframe {
        let material = material.read().unwrap();
        if let Some(rm) = render_resource_manager.get_material(material.get_id()) {
            if rm.get_edition() == material.get_edition() {
                return Some(rm.clone());
            }
        }
    }
    return convert_material(
        resource_manager,
        render_resource_manager,
        gl,
        &material,
        mode,
    );
}

fn convert_light_gizmo(
    render_resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    component: &LightComponent,
) -> Option<Arc<RenderGizmo>> {
    let id = component.get_id();
    if let Some(gizmo) = render_resource_manager.get_gizmo(id) {
        return Some(gizmo.clone());
    } else {
        let light = component.light.read().unwrap();
        if let Some(gizmo) = RenderGizmo::from_light_shape(gl, &light) {
            let gizmo = Arc::new(gizmo);
            render_resource_manager.add_gizmo(&gizmo);
            return Some(gizmo);
        }
    }
    return None;
}

fn get_gizmo_shader_program(
    render_resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    _gizmo: &Arc<RenderGizmo>,
    _mode: RenderMode,
) -> Option<Arc<RenderProgram>> {
    let id = Uuid::parse_str(GIZMO_SHADER_ID).unwrap();
    if let Some(program) = render_resource_manager.get_program(id) {
        return Some(program.clone());
    } else {
        if let Some(program) = create_render_gizmo_program(gl, id) {
            render_resource_manager.add_program(&program);
            return Some(program);
        }
    }
    None
}

fn get_light_render_gizmo(
    render_resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    component: &LightComponent,
    mode: RenderMode,
) -> Option<(Arc<RenderGizmo>, Arc<RenderMaterial>)> {
    if let Some(gizmo) = convert_light_gizmo(render_resource_manager, gl, component) {
        if let Some(program) = get_gizmo_shader_program(render_resource_manager, gl, &gizmo, mode) {
            let id = gizmo.get_id();
            if let Some(material) = render_resource_manager.get_material(id) {
                return Some((gizmo, material.clone()));
            } else {
                let mut uniform_values = Vec::new();
                uniform_values.push((
                    "base_color".to_string(),
                    RenderUniformValue::Vec4([1.0, 1.0, 0.0, 1.0]),
                ));
                let edition = Uuid::new_v4().to_string(); // Placeholder for edition, can be replaced with actual logic
                let render_material = RenderMaterial {
                    id: id,
                    edition,
                    uniform_values,
                    program,
                    gl: gl.clone(),
                };
                let render_material = Arc::new(render_material);
                render_resource_manager.add_material(&render_material);
                return Some((gizmo, render_material));
            }
        }
    }
    None
}

fn get_camera_render_gizmo(
    render_resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    component: &CameraComponent,
    mode: RenderMode,
) -> Option<(Arc<RenderGizmo>, Arc<RenderMaterial>)> {
    None
}

fn get_render_gizmo(
    render_resource_manager: &mut GLResourceManager,
    gl: &Arc<glow::Context>,
    node: &Arc<RwLock<Node>>,
    mode: RenderMode,
) -> Option<(Arc<RenderGizmo>, Arc<RenderMaterial>)> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        return get_light_render_gizmo(render_resource_manager, gl, component, mode);
    } else if let Some(component) = node.get_component::<CameraComponent>() {
        return get_camera_render_gizmo(render_resource_manager, gl, component, mode);
    }
    None
}

fn get_resource_manager(root_node: &Arc<RwLock<Node>>) -> Option<Arc<RwLock<ResourceManager>>> {
    let root_node = root_node.read().unwrap();
    if let Some(component) = root_node.get_component::<ResourceComponent>() {
        return Some(component.get_resource_manager());
    }
    None
}

fn get_gl_resource_manager(
    root_node: &Arc<RwLock<Node>>,
) -> Option<Arc<RwLock<GLResourceManager>>> {
    let root_node = root_node.read().unwrap();
    if let Some(component) = root_node.get_component::<GLResourceComponent>() {
        return Some(component.get_resource_manager());
    }
    None
}

fn get_texture_cache_manager(
    root_node: &Arc<RwLock<Node>>,
) -> Option<Arc<RwLock<TextureCacheManager>>> {
    let root_node = root_node.read().unwrap();
    if let Some(component) = root_node.get_component::<ResourceCacheComponent>() {
        return Some(component.get_texture_cache_manager());
    }
    None
}

fn build_texture_cache(
    gl: &Arc<glow::Context>,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut GLResourceManager,
    texture_cache_manager: &mut TextureCacheManager,
) {
    let mut textures = resource_manager
        .textures
        .values()
        .map(|t| {
            let order = t.read().unwrap().get_order();
            (order, t.clone())
        })
        .collect::<Vec<_>>();
    textures.sort_by_key(|(order, _)| *order);
    for (_, texture) in textures.iter() {
        let texture = texture.read().unwrap();
        let id = texture.get_id();
        if let Some(_render_texture) = render_resource_manager.get_texture(id) {
            // Texture already exists in the render resource manager
            continue;
        }
        if let Some(texture_cache) =
            texture_cache_manager.get_texture_cache(&texture, TextureCacheSize::Full)
        {
            let texture_cache = texture_cache.read().unwrap();
            let image = texture_cache.image.clone();
            if let Some(render_texture) = RenderTexture::from_image(gl, &texture, &image) {
                let render_texture = Arc::new(render_texture);
                render_resource_manager.add_texture(&render_texture);
            }
        }
    }
}

fn get_render_items_core(
    gl: &Arc<glow::Context>,
    scene_items: &[SceneItem],
    resource_manager: &ResourceManager,
    render_resource_manager: &mut GLResourceManager,
    mode: RenderMode,
) -> Vec<Arc<RenderItem>> {
    let mut render_items: Vec<Arc<RenderItem>> = Vec::new();
    for scene_item in scene_items.iter() {
        let node = scene_item.node.clone();
        let category = scene_item.category;
        let local_to_world = scene_item.matrix;
        match category {
            SceneItemType::Mesh => {
                if let Some(mesh) = get_render_mesh(render_resource_manager, gl, &node, mode) {
                    if let Some(material) = get_render_material(
                        resource_manager,
                        render_resource_manager,
                        gl,
                        &node,
                        mode,
                    ) {
                        let render_item = MeshRenderItem {
                            local_to_world,
                            mesh,
                            material,
                        };
                        render_items.push(Arc::new(RenderItem::Mesh(render_item)));
                    }
                }
            }
            SceneItemType::Light => {
                if let Some((gizmo, material)) =
                    get_render_gizmo(render_resource_manager, gl, &node, mode)
                {
                    let render_item = GizmoRenderItem {
                        local_to_world,
                        gizmo,
                        material,
                    };
                    render_items.push(Arc::new(RenderItem::Gizmo(render_item)));
                }
            }
            SceneItemType::Camera => {
                if let Some((gizmo, material)) =
                    get_render_gizmo(render_resource_manager, gl, &node, mode)
                {
                    let render_item = GizmoRenderItem {
                        local_to_world,
                        gizmo,
                        material,
                    };
                    render_items.push(Arc::new(RenderItem::Gizmo(render_item)));
                }
            }
            _ => {
                // Handle other types if needed
            }
        }
    }
    return render_items;
}

pub fn get_render_items(
    gl: &Arc<glow::Context>,
    root_node: &Arc<RwLock<Node>>,
    mode: RenderMode,
) -> Vec<Arc<RenderItem>> {
    let scene_items = get_scene_items(root_node);
    {
        let mut root_node = root_node.write().unwrap();
        if root_node.get_component_mut::<ResourceComponent>().is_none() {
            root_node.add_component::<ResourceComponent>(ResourceComponent::new());
        }
        if root_node
            .get_component_mut::<GLResourceComponent>()
            .is_none()
        {
            root_node.add_component::<GLResourceComponent>(GLResourceComponent::new(gl));
        }
        if root_node
            .get_component_mut::<ResourceCacheComponent>()
            .is_none()
        {
            root_node.add_component::<ResourceCacheComponent>(ResourceCacheComponent::new());
        }
    }
    let resource_manager = get_resource_manager(&root_node).unwrap();
    let gl_resource_manager = get_gl_resource_manager(&root_node).unwrap();
    let texture_cache_manager = get_texture_cache_manager(&root_node).unwrap();
    {
        let resource_manager = resource_manager.read().unwrap();
        let mut gl_resource_manager = gl_resource_manager.write().unwrap();
        let mut texture_cache_manager = texture_cache_manager.write().unwrap();
        build_texture_cache(
            gl,
            &resource_manager,
            &mut gl_resource_manager,
            &mut texture_cache_manager,
        );
    }
    {
        let resource_manager = resource_manager.read().unwrap();
        let mut gl_resource_manager = gl_resource_manager.write().unwrap();
        let render_items = get_render_items_core(
            gl,
            &scene_items,
            &resource_manager,
            &mut gl_resource_manager,
            mode,
        );
        return render_items;
    }
}
