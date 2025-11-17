use super::material::RenderCategory;
use super::material::RenderMaterial;
use super::material::RenderPass;
use super::material::RenderUniformValue;
use super::mesh::RenderMesh;
use super::render_item::MeshRenderItem;
use super::render_item::RenderItem;
use super::render_item::create_render_pass;
use super::render_item::get_bool;
use super::render_item::get_color;
use super::render_item::get_float;
use super::render_item::get_shader_type;
use super::render_item::get_texture;
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

use std::sync::Arc;
use std::sync::RwLock;

use bytemuck::{Pod, Zeroable};

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

fn get_base_diffuse_key(material: &Material) -> Option<String> {
    let material_type = material.get_type();
    match material_type.as_str() {
        "matte" | "plastic" | "translucent" | "uber" => {
            return "Kd".to_string().into();
        }
        "metal" => {
            return "k".to_string().into();
        }
        "glass" => {
            return "Kt".to_string().into();
        }
        "mirror" => {
            //no diffuse component
            return None;
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

/*
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

fn create_render_basic_material(
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
        let material_type = material.get_type();
        let shader_type = get_shader_type(&material_type, &uniform_values);
        let render_material = RenderMaterial {
            id,
            edition,
            material_type,
            shader_type,
            render_category: RenderCategory::Opaque,
            uniform_values,
        };
        return render_material;
    }
}

fn create_render_surface_material(
    material: &Material,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
    keys: &[&str],
) -> RenderMaterial {
    let mut uniform_values = Vec::new();
    let props = material.as_property_map();
    for key in keys {
        if let Some(color) = get_color(props, key, resource_manager) {
            uniform_values.push((key.to_string(), RenderUniformValue::Vec4(color)));
        } else if let Some(texture) =
            get_texture(props, key, resource_manager, render_resource_manager)
        {
            uniform_values.push((
                key.to_string(),
                RenderUniformValue::Texture(texture.clone()),
            ));
        } else {
            //should set default value
            uniform_values.push((
                key.to_string(),
                RenderUniformValue::Vec4([1.0, 1.0, 1.0, 1.0]),
            ));
        }
    }
    let edition = material.get_edition();
    let id = material.get_id();
    let material_type = material.get_type();
    let shader_type = get_shader_type(&material_type, &uniform_values);
    let render_material = RenderMaterial {
        id,
        edition,
        material_type,
        shader_type,
        render_category: RenderCategory::Opaque,
        uniform_values,
    };
    return render_material;
}

fn roughness_to_alpha(roughness: f32) -> f32 {
    let roughness = f32::max(roughness, 1e-3);
    let x = f32::ln(roughness);
    return 1.62142
        + 0.819955 * x
        + 0.1734 * x * x
        + 0.0171201 * x * x * x
        + 0.000640711 * x * x * x * x;
}

fn create_render_material_from_material(
    material: &Material,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> RenderMaterial {
    todo!();
    let material_type = material.get_type();
    match material_type.as_str() {
        "matte" => {
            let sigma = get_float(material.as_property_map(), "sigma").unwrap_or(0.0);
            let mut render_material = create_render_surface_material(
                material,
                resource_manager,
                render_resource_manager,
                &["Kd"],
            ); //sigma, bumpmap

            let diffuse_model = if sigma == 0.0 {
                "lambert".to_string()
            } else {
                "orennayar".to_string()
            };
            let shader_type =
                format!("matte_{}_none", diffuse_model);
            render_material.shader_type = shader_type;
            return render_material;
        }
        _ => {
            return create_render_basic_material(material, resource_manager);
        }
    }
}
*/

fn create_basic_render_passes(
    material: &Material,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Vec<RenderPass> {
    let diffuse_color = if let Some(key) = get_base_diffuse_key(material) {
        get_color(&material.props, &key, resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0])
    } else {
        [1.0, 1.0, 1.0, 1.0]
    };
    let specular_color = [1.0, 1.0, 1.0, 1.0];
    let uniform_values = vec![
        (
            "diffuse".to_string(),
            RenderUniformValue::Vec4(diffuse_color),
        ),
        (
            "specular".to_string(),
            RenderUniformValue::Vec4(specular_color),
        ),
    ];
    let render_pass = create_render_pass(
        "basic",
        RenderCategory::Opaque,
        &uniform_values,
        render_resource_manager,
    );
    return vec![render_pass];
}

fn create_matte_render_passes(
    material: &Material,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Vec<RenderPass> {
    let passes = create_basic_render_passes(material, resource_manager, render_resource_manager);
    return passes;
}

fn create_render_material_from_material(
    material: &Material,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> RenderMaterial {
    let material_type = material.get_type();
    let id = material.get_id();
    let edition = material.get_edition();
    let mut passes = vec![];
    match material_type.as_str() {
        "matte" => {
            let new_passes =
                create_matte_render_passes(material, resource_manager, render_resource_manager);
            passes.extend(new_passes);
        }
        _ => {
            let new_passes =
                create_basic_render_passes(material, resource_manager, render_resource_manager);
            passes.extend(new_passes);
        }
    }
    let render_material = RenderMaterial {
        id,
        edition,
        material_type,
        passes,
    };
    return render_material;
}

fn create_render_material_from_light(
    light: &Light,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> RenderMaterial {
    let mut uniform_values = Vec::new();
    {
        let keys = ["L", "scale"];
        for key in keys {
            if let Some(color) = get_color(light.as_property_map(), key, resource_manager) {
                uniform_values.push((key.to_string(), RenderUniformValue::Vec4(color)));
            } else {
                uniform_values.push((
                    key.to_string(),
                    RenderUniformValue::Vec4([1.0, 1.0, 1.0, 1.0]),
                ))
            }
        }
    }
    let material_type = light.get_type();
    let id = light.get_id();
    let edition = light.get_edition();
    let mut passes = vec![];
    match material_type.as_str() {
        _ => {
            //basic material
            let pass = create_render_pass(
                "arealight",
                RenderCategory::Emissive,
                &uniform_values,
                render_resource_manager,
            );
            passes.push(pass);
        }
    }
    let render_material = RenderMaterial {
        id,
        edition,
        material_type,
        passes,
    };
    return render_material;
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
        let render_material =
            create_render_material_from_light(&light, resource_manager, render_resource_manager);
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
        let render_material = create_render_material_from_material(
            &material,
            resource_manager,
            render_resource_manager,
        );
        let render_material = Arc::new(render_material);

        //let material_type = render_material.get_material_type();
        //let shader_type = render_material.get_shader_type();
        //if material_type == "plastic" {
        //    println!("Created render material: type={}, shader={}", material_type, shader_type);
        //}

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
