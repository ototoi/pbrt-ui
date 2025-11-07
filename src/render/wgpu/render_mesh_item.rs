use super::material::RenderCategory;
use super::material::RenderMaterial;
use super::material::RenderUniformValue;
use super::mesh::RenderMesh;
use super::render_item::MeshRenderItem;
use super::render_item::RenderItem;
use super::render_item::get_color;
use super::render_resource::RenderResourceManager;
use crate::model::scene::Light;
use crate::model::scene::Material;
use crate::model::scene::MaterialComponent;
use crate::model::scene::Node;

use crate::model::scene::LightComponent;
use crate::model::scene::ResourceManager;
use crate::model::scene::ShapeComponent;
use crate::render::render_mode::RenderMode;
use crate::render::scene_item::*;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;

use eframe::wgpu;

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

fn get_base_color_key(material: &Material) -> Option<String> {
    let material_type = material.get_type();
    match material_type.as_str() {
        "matte" | "plastic" | "translucent" | "uber" => {
            return "Kd".to_string().into();
        }
        "metal" => {
            return "k".to_string().into();
        }
        "glass" | "mirror" => {
            return "Kr".to_string().into();
        }
        "substrate" => {
            return "Kd".to_string().into();
        }
        "kdsubsurface" => {
            return "Kd".to_string().into();
        }
        "disney" => {
            return "color".to_string().into();
        }
        _ => {}
    }
    return None;
}

fn get_base_color_value(
    material: &Material,
    key: &str,
    resource_manager: &ResourceManager,
) -> Option<RenderUniformValue> {
    let props = material.as_property_map();
    if let Some(color) = get_color(props, key, resource_manager) {
        return Some(RenderUniformValue::Vec4(color));
    }
    return None;
}

fn create_render_material_from_material(
    material: &Material,
    resource_manager: &ResourceManager,
) -> RenderMaterial {
    let mut uniform_values_map = HashMap::new();
    uniform_values_map.insert(
        "base_color".to_string(),
        RenderUniformValue::Vec4([1.0, 1.0, 1.0, 1.0]),
    );
    if let Some(base_color_key) = get_base_color_key(material) {
        if let Some(value) = get_base_color_value(material, &base_color_key, resource_manager) {
            uniform_values_map.insert("base_color".to_string(), value.clone());
        }
    }
    {
        let keys = ["Kd", "Ks", "Kt", "Kr"];
        for key in keys {
            if let Some(value) = get_base_color_value(material, key, resource_manager) {
                uniform_values_map.insert(key.to_string(), value.clone());
            }
        }
    }
    {
        let uniform_values: Vec<(String, RenderUniformValue)> = uniform_values_map
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let edition = material.get_edition();
        let id = material.get_id();
        let ty = material.get_type();
        let render_material = RenderMaterial {
            id,
            edition,
            ty,
            render_category: RenderCategory::Opaque,
            uniform_values,
        };
        return render_material;
    }
}

fn create_render_material_from_light(
    light: &Light,
    resource_manager: &ResourceManager,
) -> RenderMaterial {
    let mut uniform_values_map = HashMap::new();
    {
        let keys = ["L", "scale"];
        for key in keys {
            if let Some(color) = get_color(light.as_property_map(), key, resource_manager) {
                uniform_values_map.insert(key.to_string(), RenderUniformValue::Vec4(color));
            }
        }
    }
    {
        let uniform_values: Vec<(String, RenderUniformValue)> = uniform_values_map
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let edition = light.get_edition();
        let id = light.get_id();
        let ty = format!("area_light_{}", light.get_type());
        let render_material = RenderMaterial {
            id,
            edition,
            ty,
            render_category: RenderCategory::Emissive,
            uniform_values,
        };
        return render_material;
    }
}

pub fn get_render_material(
    node: &Arc<RwLock<Node>>,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<Arc<RenderMaterial>> {
    let node = node.read().unwrap();
    if let Some(light) = node.get_component::<LightComponent>() {
        //for area light
        let light = light.get_light();
        let light = light.read().unwrap();
        let light_id = light.get_id();
        if let Some(mat) = render_resource_manager.get_material(light_id) {
            if mat.edition == light.get_edition() {
                return Some(mat.clone());
            }
        }
        let render_material = create_render_material_from_light(&light, resource_manager);
        let render_material = Arc::new(render_material);
        render_resource_manager.add_material(&render_material);
        return Some(render_material);
    } else if let Some(component) = node.get_component::<MaterialComponent>() {
        //for surface material
        let material = component.get_material();
        let material = material.read().unwrap();
        let material_id = material.get_id();
        if let Some(mat) = render_resource_manager.get_material(material_id) {
            if mat.edition == material.get_edition() {
                return Some(mat.clone());
            }
        }
        let render_material = create_render_material_from_material(&material, resource_manager);
        let render_material = Arc::new(render_material);
        render_resource_manager.add_material(&render_material);
        return Some(render_material);
    }
    return None;
}

pub fn get_render_mesh_item(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    item: &SceneItem,
    mode: RenderMode,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    if let Some(mesh) = get_mesh(device, queue, &item.node, render_resource_manager) {
        let matrix = glam::Mat4::from(item.matrix);
        let material = if mode == RenderMode::Lighting {
            get_render_material(&item.node, &resource_manager, render_resource_manager)
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
