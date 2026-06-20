use super::material::RenderCategory;
use super::material::RenderMaterial;
use super::material::RenderPass;
use super::material::RenderUniformValue;
use super::render_item::create_render_pass;
use super::render_item::get_bool;
use super::render_item::get_color;
use super::render_item::get_float;
use super::render_resource::RenderResourceManager;
use super::render_texture_cache::get_texture;
use crate::model::scene::Light;
use crate::model::scene::LightComponent;
use crate::model::scene::Material;
use crate::model::scene::MaterialComponent;
use crate::model::scene::Node;
use crate::model::scene::ResourceManager;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::wgpu;

fn get_base_diffuse_key(material: &Material) -> Option<String> {
    match material.get_type().as_str() {
        "matte" | "plastic" | "translucent" | "uber" => Some("Kd".to_string()),
        "metal" => Some("k".to_string()),
        "glass" => Some("Kt".to_string()),
        "mirror" => None,
        "substrate" => Some("Kd".to_string()),
        "kdsubsurface" => Some("Kd".to_string()),
        "disney" => Some("color".to_string()),
        _ => None,
    }
}

fn roughness_to_alpha(roughness: f32) -> f32 {
    let roughness = f32::max(roughness, 1e-3);
    let x = f32::ln(roughness);
    1.62142
        + 0.819955 * x
        + 0.1734 * x * x
        + 0.0171201 * x * x * x
        + 0.000640711 * x * x * x * x
}

fn create_basic_render_passes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    material: &Material,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Vec<Arc<RenderPass>> {
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
    vec![create_render_pass(
        device,
        queue,
        "basic",
        RenderCategory::Opaque,
        &uniform_values,
        "ggx",
        render_resource_manager,
    )]
}

fn create_matte_render_passes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    material: &Material,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Vec<Arc<RenderPass>> {
    let keys = ["Kd"];
    let mut uniform_values = vec![];
    for key in keys {
        if let Some(color) = get_color(&material.props, key, resource_manager) {
            uniform_values.push((key.to_lowercase(), RenderUniformValue::Vec4(color)));
        } else if let Some(texture) = get_texture(
            &material.props,
            key,
            resource_manager,
            render_resource_manager,
        ) {
            uniform_values.push((key.to_lowercase(), RenderUniformValue::Texture(texture)));
        } else {
            uniform_values.push((
                key.to_lowercase(),
                RenderUniformValue::Vec4([1.0, 1.0, 1.0, 1.0]),
            ));
        }
    }
    if let Some(texture) = get_texture(
        &material.props,
        "bumpmap",
        resource_manager,
        render_resource_manager,
    ) {
        uniform_values.push(("bumpmap".to_string(), RenderUniformValue::Texture(texture)));
    } else {
        uniform_values.push((
            "bumpmap".to_string(),
            RenderUniformValue::Vec4([1.0, 1.0, 0.0, 0.0]),
        ));
    }
    let mut shader_type = "lambertian".to_string();
    let mut ltc_type = "microfacet_reflection".to_string();
    let sigma = get_float(&material.props, "sigma").unwrap_or(0.0);
    if sigma > 0.0 {
        let sigma = (sigma.to_radians() / (0.5 * std::f32::consts::PI)).clamp(0.0, 1.0);
        uniform_values.push(("sigma".to_string(), RenderUniformValue::Float(sigma)));
        shader_type = "oren_nayar".to_string();
        ltc_type = "oren_nayar".to_string();
    }
    vec![create_render_pass(
        device,
        queue,
        &shader_type,
        RenderCategory::Opaque,
        &uniform_values,
        &ltc_type,
        render_resource_manager,
    )]
}

fn create_plastic_render_passes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    material: &Material,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Vec<Arc<RenderPass>> {
    let keys = ["Kd", "Ks"];
    let mut uniform_values = vec![];
    for key in keys {
        if let Some(color) = get_color(&material.props, key, resource_manager) {
            uniform_values.push((key.to_lowercase(), RenderUniformValue::Vec4(color)));
        } else if let Some(texture) = get_texture(
            &material.props,
            key,
            resource_manager,
            render_resource_manager,
        ) {
            uniform_values.push((key.to_lowercase(), RenderUniformValue::Texture(texture)));
        } else {
            uniform_values.push((
                key.to_lowercase(),
                RenderUniformValue::Vec4([1.0, 1.0, 1.0, 1.0]),
            ));
        }
    }
    let mut roughness = get_float(&material.props, "roughness").unwrap_or(0.1);
    if get_bool(&material.props, "remaproughness").unwrap_or(true) {
        roughness = roughness_to_alpha(roughness);
    }
    uniform_values.push((
        "roughness".to_string(),
        RenderUniformValue::Float(roughness),
    ));
    if let Some(texture) = get_texture(
        &material.props,
        "bumpmap",
        resource_manager,
        render_resource_manager,
    ) {
        uniform_values.push(("bumpmap".to_string(), RenderUniformValue::Texture(texture)));
    } else {
        uniform_values.push((
            "bumpmap".to_string(),
            RenderUniformValue::Vec4([1.0, 1.0, 0.0, 0.0]),
        ));
    }
    vec![create_render_pass(
        device,
        queue,
        "plastic",
        RenderCategory::Opaque,
        &uniform_values,
        "microfacet_reflection",
        render_resource_manager,
    )]
}

fn create_uber_render_passes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    material: &Material,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Vec<Arc<RenderPass>> {
    create_plastic_render_passes(
        device,
        queue,
        material,
        resource_manager,
        render_resource_manager,
    )
}

fn create_substrate_render_passes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    material: &Material,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Vec<Arc<RenderPass>> {
    create_plastic_render_passes(
        device,
        queue,
        material,
        resource_manager,
        render_resource_manager,
    )
}

fn create_glass_render_passes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    material: &Material,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Vec<Arc<RenderPass>> {
    let kt = get_color(&material.props, "Kt", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let kr = get_color(&material.props, "Kr", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let eta = get_float(&material.props, "eta").unwrap_or(1.5);
    let mut roughness = get_float(&material.props, "uroughness").unwrap_or(0.1);
    if get_bool(&material.props, "remaproughness").unwrap_or(true) {
        roughness = roughness_to_alpha(roughness);
    }
    vec![
        create_render_pass(
            device,
            queue,
            "glass_transmission",
            RenderCategory::Transparent,
            &vec![
                ("kt".to_string(), RenderUniformValue::Vec4(kt)),
                ("roughness".to_string(), RenderUniformValue::Float(roughness)),
                ("eta".to_string(), RenderUniformValue::Float(eta)),
            ],
            "microfacet_reflection",
            render_resource_manager,
        ),
        create_render_pass(
            device,
            queue,
            "glass_reflection",
            RenderCategory::TransparentSpecular,
            &vec![
                ("kr".to_string(), RenderUniformValue::Vec4(kr)),
                ("roughness".to_string(), RenderUniformValue::Float(roughness)),
                ("eta".to_string(), RenderUniformValue::Float(eta)),
            ],
            "microfacet_reflection",
            render_resource_manager,
        ),
    ]
}

fn create_metal_render_passes(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    material: &Material,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Vec<Arc<RenderPass>> {
    let keys = ["eta", "k"];
    let mut uniform_values = vec![];
    for key in keys {
        if let Some(color) = get_color(&material.props, key, resource_manager) {
            uniform_values.push((key.to_lowercase(), RenderUniformValue::Vec4(color)));
        } else if let Some(texture) = get_texture(
            &material.props,
            key,
            resource_manager,
            render_resource_manager,
        ) {
            uniform_values.push((key.to_lowercase(), RenderUniformValue::Texture(texture)));
        } else {
            uniform_values.push((
                key.to_lowercase(),
                RenderUniformValue::Vec4([1.0, 1.0, 1.0, 1.0]),
            ));
        }
    }
    let mut roughness = get_float(&material.props, "roughness").unwrap_or(0.1);
    if get_bool(&material.props, "remaproughness").unwrap_or(true) {
        roughness = roughness_to_alpha(roughness);
    }
    uniform_values.push((
        "roughness".to_string(),
        RenderUniformValue::Float(roughness),
    ));
    if let Some(texture) = get_texture(
        &material.props,
        "bumpmap",
        resource_manager,
        render_resource_manager,
    ) {
        uniform_values.push(("bumpmap".to_string(), RenderUniformValue::Texture(texture)));
    } else {
        uniform_values.push((
            "bumpmap".to_string(),
            RenderUniformValue::Vec4([1.0, 1.0, 0.0, 0.0]),
        ));
    }
    vec![create_render_pass(
        device,
        queue,
        "metal",
        RenderCategory::Opaque,
        &uniform_values,
        "microfacet_reflection",
        render_resource_manager,
    )]
}

fn create_render_material_from_material(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    material: &Material,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> RenderMaterial {
    let material_type = material.get_type();
    let passes = match material_type.as_str() {
        "matte" => create_matte_render_passes(
            device,
            queue,
            material,
            resource_manager,
            render_resource_manager,
        ),
        "plastic" => create_plastic_render_passes(
            device,
            queue,
            material,
            resource_manager,
            render_resource_manager,
        ),
        "uber" => create_uber_render_passes(
            device,
            queue,
            material,
            resource_manager,
            render_resource_manager,
        ),
        "substrate" => create_substrate_render_passes(
            device,
            queue,
            material,
            resource_manager,
            render_resource_manager,
        ),
        "glass" => create_glass_render_passes(
            device,
            queue,
            material,
            resource_manager,
            render_resource_manager,
        ),
        "metal" => create_metal_render_passes(
            device,
            queue,
            material,
            resource_manager,
            render_resource_manager,
        ),
        _ => create_basic_render_passes(
            device,
            queue,
            material,
            resource_manager,
            render_resource_manager,
        ),
    };
    RenderMaterial {
        id: material.get_id(),
        edition: material.get_edition(),
        material_type,
        passes,
    }
}

fn create_render_material_from_light(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    light: &Light,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> RenderMaterial {
    let mut uniform_values = Vec::new();
    for key in ["L", "scale"] {
        if let Some(color) = get_color(light.as_property_map(), key, resource_manager) {
            uniform_values.push((key.to_string(), RenderUniformValue::Vec4(color)));
        } else {
            uniform_values.push((
                key.to_string(),
                RenderUniformValue::Vec4([1.0, 1.0, 1.0, 1.0]),
            ));
        }
    }
    let material_type = light.get_type();
    let passes = vec![create_render_pass(
        device,
        queue,
        "arealight_diffuse",
        RenderCategory::Emissive,
        &uniform_values,
        "",
        render_resource_manager,
    )];
    RenderMaterial {
        id: light.get_id(),
        edition: light.get_edition(),
        material_type,
        passes,
    }
}

pub fn get_render_material(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    node: &Arc<RwLock<Node>>,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<Arc<RenderMaterial>> {
    let node = node.read().unwrap();
    if let Some(light) = node.get_component::<LightComponent>() {
        let light = light.get_light();
        let light = light.read().unwrap();
        let light_id = light.get_id();
        if let Some(mat) = render_resource_manager.get_material(light_id)
            && mat.edition == light.get_edition()
        {
            return Some(mat.clone());
        }
        let render_material = Arc::new(create_render_material_from_light(
            device,
            queue,
            &light,
            resource_manager,
            render_resource_manager,
        ));
        render_resource_manager.add_material(&render_material);
        return Some(render_material);
    }
    if let Some(component) = node.get_component::<MaterialComponent>() {
        let material = component.get_material();
        let material = material.read().unwrap();
        let material_id = material.get_id();
        if let Some(mat) = render_resource_manager.get_material(material_id)
            && mat.edition == material.get_edition()
        {
            return Some(mat.clone());
        }
        let render_material = Arc::new(create_render_material_from_material(
            device,
            queue,
            &material,
            resource_manager,
            render_resource_manager,
        ));
        render_resource_manager.add_material(&render_material);
        return Some(render_material);
    }
    None
}
