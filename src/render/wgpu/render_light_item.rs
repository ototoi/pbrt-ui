use super::light::RenderLight;
use super::light::RenderLightType;
use super::lines::RenderLines;
use super::material::RenderMaterial;
use super::material::RenderUniformValue;
use super::render_item::LinesRenderItem;
use super::render_item::RenderItem;
use super::render_item::RenderLightItem;
use super::render_item::get_color;
use super::render_resource::RenderResourceManager;
use crate::conversion::light_shape::create_light_shape;
use crate::model::base::Matrix4x4;
use crate::model::base::Vector3;
use crate::model::scene::LightComponent;
use crate::model::scene::Node;
use crate::model::scene::ResourceManager;
use crate::render::render_mode::RenderMode;
use crate::render::scene_item::*;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::wgpu;
use uuid::Uuid;

#[inline]
fn coordinate_system(v1: &Vector3) -> (Vector3, Vector3) {
    let v2 = if f32::abs(v1.x) > f32::abs(v1.y) {
        Vector3::new(-v1.z, 0.0, v1.x) / f32::sqrt(v1.x * v1.x + v1.z * v1.z)
    } else {
        Vector3::new(0.0, v1.z, -v1.y) / f32::sqrt(v1.y * v1.y + v1.z * v1.z)
    };
    let v3 = Vector3::cross(v1, &v2).normalize();
    return (v2, v3);
}

fn get_light_type(node: &Arc<RwLock<Node>>) -> Option<String> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        let light = component.get_light();
        let light = light.read().unwrap();
        return Some(light.get_type());
    }
    return None; // No LightComponent found
}

fn get_directional_light_item(
    item: &SceneItem,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    let node = &item.node;
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        let light = component.get_light();
        let light = light.read().unwrap();

        let id = light.get_id();
        let light_type = light.get_type();
        let edition = light.get_edition();
        if let Some(render_light) = render_resource_manager.get_light(id) {
            if render_light.edition == edition {
                let render_item = RenderLightItem {
                    light: render_light.clone(),
                    matrix: glam::Mat4::from(item.matrix),
                };
                return Some(RenderItem::Light(render_item));
            }
        }
        assert!(
            light_type == "distant",
            "Expected light type to be 'distant', found: {}",
            light_type
        );

        let props = light.as_property_map();

        let mut from = props.get_floats("from");
        if from.len() != 3 {
            from = vec![0.0, 0.0, 0.0];
        }
        let mut to = props.get_floats("to");
        if to.len() != 3 {
            to = vec![0.0, 0.0, 1.0];
        }
        let from = Vector3::new(from[0], from[1], from[2]);
        let to = Vector3::new(to[0], to[1], to[2]);
        let dir = to - from;
        let direction = [dir.x, dir.y, dir.z];

        let l = get_color(&props, "L", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let scale = get_color(&props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

        let intensity = [l[0] * scale[0], l[1] * scale[1], l[2] * scale[2]];
        let render_light = RenderLight {
            id,
            edition: edition.clone(),
            light_type: RenderLightType::Directional,
            direction: direction,
            intensity: intensity,
            ..Default::default()
        };
        let render_light = Arc::new(render_light);
        render_resource_manager.add_light(&render_light);
        let render_item = RenderLightItem {
            light: render_light.clone(),
            matrix: glam::Mat4::from(item.matrix),
        };
        return Some(RenderItem::Light(render_item));
    }
    return None;
}

fn get_point_light_item(
    item: &SceneItem,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    let node = &item.node;
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        let light = component.get_light();
        let light = light.read().unwrap();

        let id = light.get_id();
        let light_type = light.get_type();
        let edition = light.get_edition();
        if let Some(render_light) = render_resource_manager.get_light(id) {
            if render_light.edition == edition {
                let render_item = RenderLightItem {
                    light: render_light.clone(),
                    matrix: glam::Mat4::from(item.matrix),
                };
                return Some(RenderItem::Light(render_item));
            }
        }
        assert!(
            light_type == "point",
            "Expected light type to be 'point', found: {}",
            light_type
        );
        let props = light.as_property_map();

        let mut from = props.get_floats("from");
        if from.len() != 3 {
            from = vec![0.0, 0.0, 0.0];
        }

        let translation = Matrix4x4::translate(from[0], from[1], from[2]);
        let mat = translation * item.matrix;

        let l = get_color(&props, "I", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let scale = get_color(&props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

        let intensity = [l[0] * scale[0], l[1] * scale[1], l[2] * scale[2]];
        let radius = 10.0; //todo: get radius from properties
        let render_light = RenderLight {
            id,
            edition: edition.clone(),
            light_type: RenderLightType::Point,
            intensity: intensity,
            range: [0.0, radius],
            ..Default::default()
        };
        let render_light = Arc::new(render_light);
        render_resource_manager.add_light(&render_light);

        let render_item = RenderLightItem {
            light: render_light.clone(),
            matrix: glam::Mat4::from(mat),
        };
        return Some(RenderItem::Light(render_item));
    }
    return None; // Point lights are not yet supported
}

fn get_spot_light_item(
    item: &SceneItem,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    let node = &item.node;
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        let light = component.get_light();
        let light = light.read().unwrap();

        let id = light.get_id();
        let light_type = light.get_type();
        let edition = light.get_edition();
        if let Some(render_light) = render_resource_manager.get_light(id) {
            if render_light.edition == edition {
                let render_item = RenderLightItem {
                    light: render_light.clone(),
                    matrix: glam::Mat4::from(item.matrix),
                };
                return Some(RenderItem::Light(render_item));
            }
        }
        assert!(
            light_type == "spot",
            "Expected light type to be 'point', found: {}",
            light_type
        );
        let props = light.as_property_map();

        let mut from = props.get_floats("from");
        if from.len() != 3 {
            from = vec![0.0, 0.0, 0.0];
        }
        let mut to = props.get_floats("to");
        if to.len() != 3 {
            to = vec![0.0, 0.0, 1.0];
        }
        let from = Vector3::new(from[0], from[1], from[2]);
        let to = Vector3::new(to[0], to[1], to[2]);
        let dir = (to - from).normalize();
        let (du, dv) = coordinate_system(&dir);
        let dir_to_z = Matrix4x4::new(
            du.x, du.y, du.z, 0.0, dv.x, dv.y, dv.z, 0., dir.x, dir.y, dir.z, 0.0, 0.0, 0.0, 0.0,
            1.0,
        );
        let l2w = item.matrix;
        let mat = l2w
            * Matrix4x4::translate(from.x, from.y, from.z)
            * Matrix4x4::inverse(&dir_to_z).unwrap();

        let coneangle = props.find_one_float("coneangle").unwrap_or(30.0);
        let conedelta = props.find_one_float("conedelta").unwrap_or(5.0); //5.0
        let conedelta = props.find_one_float("conedeltaangle").unwrap_or(conedelta);
        let conedelta = conedelta.clamp(0.0, coneangle);

        let outer_angle = f32::to_radians(coneangle);
        let inner_angle = f32::to_radians((coneangle - conedelta).max(0.0));

        let l = get_color(&props, "I", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let scale = get_color(&props, "scale", resource_manager).unwrap_or([1.0, 1.0, 1.0, 1.0]);

        let intensity = [l[0] * scale[0], l[1] * scale[1], l[2] * scale[2]];
        let radius = 10.0; //todo: get radius from properties
        let render_light = RenderLight {
            id,
            edition: edition.clone(),
            light_type: RenderLightType::Spot,
            position: [0.0, 0.0, 0.0], // Position is not used for spot lights
            direction: [0.0, 0.0, 1.0], // Direction is not used for spot lights
            intensity: intensity,
            range: [0.0, radius],
            angle: [inner_angle, outer_angle],
            ..Default::default()
        };
        let render_light = Arc::new(render_light);
        render_resource_manager.add_light(&render_light);

        let render_item = RenderLightItem {
            light: render_light.clone(),
            matrix: glam::Mat4::from(mat),
        };
        return Some(RenderItem::Light(render_item));
    }
    return None; // Point lights are not yet supported
}

fn get_lines_material(
    id: Uuid,
    edition: &str,
    render_resource_manager: &mut RenderResourceManager,
    base_color: &[f32; 4],
) -> Option<Arc<RenderMaterial>> {
    if let Some(mat) = render_resource_manager.get_material(id) {
        if mat.edition == edition {
            return Some(mat.clone());
        }
    }
    // Create a default material for the light gizmo
    let mut uniform_values = Vec::new();
    uniform_values.push((
        "base_color".to_string(),
        RenderUniformValue::Vec4(base_color.clone()),
    ));
    let render_material = RenderMaterial {
        id: id,
        edition: edition.to_string(),
        uniform_values,
    };
    let render_material = Arc::new(render_material);
    render_resource_manager.add_material(&render_material);
    return Some(render_material);
}

fn get_light_gizmo_material(
    node: &Arc<RwLock<Node>>,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<Arc<RenderMaterial>> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<LightComponent>() {
        let light = component.get_light();
        let light = light.read().unwrap();
        let light_id = light.get_id();
        let edition = light.get_edition();
        let base_color = [1.0, 1.0, 0.0, 1.0]; // Default Yellow color for light gizmo
        return get_lines_material(light_id, &edition, render_resource_manager, &base_color);
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
        let light_edition = light.get_edition();
        if let Some(lines) = render_resource_manager.get_lines(light_id) {
            if lines.edition == light_edition {
                return Some(lines.clone());
            }
        }
        if let Some(light_shape) = create_light_shape(&light) {
            let lines = &light_shape.lines;
            let lines = lines
                .iter()
                .map(|line| {
                    line.iter()
                        .map(|point| [point.x, point.y, point.z])
                        .collect::<Vec<[f32; 3]>>()
                })
                .collect::<Vec<Vec<[f32; 3]>>>();
            if let Some(lines) =
                RenderLines::from_lines(device, queue, light_id, &light_edition, &lines)
            {
                let lines = Arc::new(lines);
                render_resource_manager.add_lines(&lines);
                return Some(lines);
            }
        }
    }
    return None;
}

pub fn get_render_light_item(
    _device: &wgpu::Device,
    _queue: &wgpu::Queue,
    item: &SceneItem,
    _mode: RenderMode,
    resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    if let Some(light_type) = get_light_type(&item.node) {
        match light_type.as_str() {
            "distant" => {
                return get_directional_light_item(item, resource_manager, render_resource_manager);
            }
            "point" => {
                return get_point_light_item(item, resource_manager, render_resource_manager);
            }
            "spot" => {
                return get_spot_light_item(item, resource_manager, render_resource_manager); // Spot lights are not yet supported
            }
            _ => {
                // Handle unknown or unsupported light types
                return None;
            }
        }
    }
    return None; // Placeholder for light retrieval logic
}

pub fn get_render_light_gizmo_item(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    item: &SceneItem,
    _mode: RenderMode,
    _resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<RenderItem> {
    if let Some(lines) = get_light_gizmo(device, queue, &item.node, render_resource_manager) {
        let matrix = glam::Mat4::from(item.matrix);
        let material = get_light_gizmo_material(&item.node, render_resource_manager);
        let render_item = LinesRenderItem {
            lines,
            material,
            matrix,
        };
        return Some(RenderItem::Lines(render_item));
    }
    return None;
}
