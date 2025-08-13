use super::lines::RenderLines;
use super::material::RenderMaterial;
use super::material::RenderUniformValue;
use super::mesh::RenderMesh;
use super::render_resource::RenderResourceComponent;
use super::render_resource::RenderResourceManager;
use crate::conversion::light_shape::create_light_shape;
use crate::model::base::Property;
use crate::model::scene::CoordinateSystemComponent;
use crate::model::scene::LightComponent;
use crate::model::scene::Material;
use crate::model::scene::MaterialComponent;
use crate::model::scene::Node;
use crate::model::scene::ShapeComponent;
use crate::render::render_mode::RenderMode;
use crate::render::scene_item::*;
use crate::render::wgpu::lines;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::wgpu;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct MeshRenderItem {
    pub mesh: Arc<RenderMesh>,
    pub material: Option<Arc<RenderMaterial>>,
    pub matrix: glam::Mat4,
}

#[derive(Debug, Clone)]
pub struct LinesRenderItem {
    pub lines: Arc<RenderLines>,
    pub material: Option<Arc<RenderMaterial>>,
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

fn get_base_color_value(material: &Material, key: &str) -> Option<RenderUniformValue> {
    let props = material.as_property_map();
    if let Some((_key_type, _key_name, value)) = props.entry(key) {
        if let Property::Floats(v) = value {
            if v.len() >= 3 {
                return Some(RenderUniformValue::Vec4([v[0], v[1], v[2], 1.0]));
            }
        }
    }
    return None;
}

fn create_shaded_material(material: &Material) -> RenderMaterial {
    if let Some(base_color_key) = get_base_color_key(material) {
        if let Some(value) = get_base_color_value(material, &base_color_key) {
            let mut uniform_values = Vec::new();
            uniform_values.push(("base_color".to_string(), value.clone()));
            uniform_values.push((base_color_key.to_string(), value.clone()));
            let edition = material.get_edition();
            let id = material.get_id();
            let render_material = RenderMaterial {
                id,
                edition,
                uniform_values,
            };
            return render_material;
        }
    }
    {
        // Fallback to a default solid material if no base color is found
        let mut uniform_values = Vec::new();
        uniform_values.push((
            "base_color".to_string(),
            RenderUniformValue::Vec4([1.0, 0.0, 1.0, 1.0]),
        ));
        let edition = material.get_edition();
        let id = material.get_id();
        let render_material = RenderMaterial {
            id,
            edition,
            uniform_values,
        };
        return render_material;
    }
}

pub fn get_surface_material(
    node: &Arc<RwLock<Node>>,
    render_resource_manager: &mut RenderResourceManager,
) -> Option<Arc<RenderMaterial>> {
    let node = node.read().unwrap();
    if let Some(component) = node.get_component::<MaterialComponent>() {
        let material = component.get_material();
        let material = material.read().unwrap();
        let material_id = material.get_id();
        if let Some(mat) = render_resource_manager.get_material(material_id) {
            if mat.edition == material.get_edition() {
                return Some(mat.clone());
            }
        }
        let render_material = create_shaded_material(&material);
        let render_material = Arc::new(render_material);
        render_resource_manager.add_material(&render_material);
        return Some(render_material);
    }
    return None;
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
                    let material = match mode {
                        RenderMode::Wire => None,
                        RenderMode::Solid => None,
                        RenderMode::Lighting => {
                            get_surface_material(&item.node, &mut resource_manager)
                        }
                    };
                    let render_item = MeshRenderItem {
                        mesh,
                        material,
                        matrix,
                    };
                    render_items.push(Arc::new(RenderItem::Mesh(render_item)));
                }
            }
            SceneItemType::Light => {
                if let Some(lines) =
                    get_light_gizmo(device, queue, &item.node, &mut resource_manager)
                {
                    let matrix = glam::Mat4::from(item.matrix);
                    let material = get_light_gizmo_material(&item.node, &mut resource_manager);
                    let render_item = LinesRenderItem {
                        lines,
                        material,
                        matrix,
                    };
                    render_items.push(Arc::new(RenderItem::Lines(render_item)));
                }
            }
            // Handle other categories like Light, Camera, etc.
            _ => {}
        }
    }
    //additional render items based on the mode
    {
        let display_world_axes = true; // This should be a setting or parameter
        if display_world_axes {
            const IDS: [Uuid; 3] = [
                Uuid::from_u128(0x00000000_1000_0000_0000_000000000001), // X Axis
                Uuid::from_u128(0x00000000_1000_0000_0000_000000000002), // Y Axis
                Uuid::from_u128(0x00000000_1000_0000_0000_000000000003), // Z Axis
            ];
            for i in 0..3 {
                let id = IDS[i];
                let edition = "world_axes".to_string();
                let mut render_lines = None;
                if let Some(lines) = resource_manager.get_lines(id) {
                    render_lines = Some(lines.clone());
                } else {
                    let mut line = vec![];
                    let scale = 1000.0f32; // Scale factor for the axes
                    match i {
                        0 => {
                            // X Axis
                            line.push([-scale, 0.0, 0.0]);
                            line.push([scale, 0.0, 0.0]);
                        }
                        1 => {
                            // Y Axis
                            line.push([0.0, -scale, 0.0]);
                            line.push([0.0, scale, 0.0]);
                        }
                        2 => {
                            // Z Axis
                            line.push([0.0, 0.0, -scale]);
                            line.push([0.0, 0.0, scale]);
                        }
                        _ => continue,
                    }
                    let lines = vec![line];
                    if let Some(lines) =
                        RenderLines::from_lines(device, queue, id, &edition, &lines)
                    {
                        let lines = Arc::new(lines);
                        resource_manager.add_lines(&lines);
                        render_lines = Some(lines);
                    }
                }
                if let Some(lines) = render_lines {
                    let color = match i {
                        0 => [1.0, 0.0, 0.0, 1.0], // Red for X
                        1 => [0.0, 1.0, 0.0, 1.0], // Green for Y
                        2 => [0.0, 0.0, 1.0, 1.0], // Blue for Z
                        _ => continue,
                    };
                    let matrix = glam::Mat4::IDENTITY; // World axes are at the origin
                    let material = get_lines_material(id, &edition, &mut resource_manager, &color);
                    let render_item = LinesRenderItem {
                        lines,
                        material,
                        matrix,
                    };
                    render_items.push(Arc::new(RenderItem::Lines(render_item)));
                }
            }
        }
        let display_grid = true; // This should be a setting or parameter
        if display_grid {
            const ID: Uuid = Uuid::from_u128(0x00000000_1000_0000_0000_000000000004); // Unique ID for the grid
            const GRID_SIZE: f32 = 1000.0; // Size of the grid
            const GRID_STEP: f32 = 10.0; // Step size for grid lines
            enum PlaneType {
                XY,
                ZX,
                YZ,
            }
            let mut plane_type = PlaneType::XY; // This should be a setting or parameter
            {
                let node = node.read().unwrap();
                if let Some(component) = node.get_component::<CoordinateSystemComponent>() {
                    let up = component.get_up_vector();
                    let up = [up.x.abs(), up.y.abs(), up.z.abs()];
                    let mut max_axis = 0;
                    for (i, &value) in up.iter().enumerate() {
                        if value > up[max_axis] {
                            max_axis = i;
                        }
                    }
                    match max_axis {
                        0 => plane_type = PlaneType::YZ, // X is largest, use YZ plane
                        1 => plane_type = PlaneType::ZX, // Y is largest, use ZX plane
                        2 => plane_type = PlaneType::XY, // Z is largest, use XY plane
                        _ => {}
                    }
                }
            }

            let id = ID;
            let edition = "grid".to_string();
            let mut render_lines = None;
            if let Some(lines) = resource_manager.get_lines(id) {
                render_lines = Some(lines.clone());
            } else {
                let mut lines = vec![];
                for i in (-GRID_SIZE as i32..=GRID_SIZE as i32).step_by(GRID_STEP as usize) {
                    // Horizontal lines
                    lines.push(vec![[i as f32, -GRID_SIZE], [i as f32, GRID_SIZE]]);
                    // Vertical lines
                    lines.push(vec![[-GRID_SIZE, i as f32], [GRID_SIZE, i as f32]]);
                }
                let lines: Vec<Vec<[f32; 3]>> = match plane_type {
                    PlaneType::XY => {
                        // XY plane, no swap needed
                        lines
                            .into_iter()
                            .map(|line| {
                                line.into_iter()
                                    .map(|point| [point[0], point[1], 0.0])
                                    .collect()
                            })
                            .collect()
                    }
                    PlaneType::ZX => {
                        // ZX plane, swap X and Y
                        lines
                            .into_iter()
                            .map(|line| {
                                line.into_iter()
                                    .map(|point| [point[1], 0.0, point[0]])
                                    .collect()
                            })
                            .collect()
                    }
                    PlaneType::YZ => {
                        // YZ plane, swap X and Z
                        lines
                            .into_iter()
                            .map(|line| {
                                line.into_iter()
                                    .map(|point| [0.0, point[0], point[1]])
                                    .collect()
                            })
                            .collect()
                    }
                };

                if let Some(lines) = RenderLines::from_lines(device, queue, id, &edition, &lines) {
                    let lines = Arc::new(lines);
                    resource_manager.add_lines(&lines);
                    render_lines = Some(lines);
                }
            }
            if let Some(lines) = render_lines {
                let color = [0.5, 0.5, 0.5, 1.0]; // Gray color for the grid
                let material = get_lines_material(id, &edition, &mut resource_manager, &color);
                let matrix = glam::Mat4::IDENTITY; // Grid is at the origin
                let render_item = LinesRenderItem {
                    lines,
                    material,
                    matrix,
                };
                render_items.push(Arc::new(RenderItem::Lines(render_item)));
            }
        }
    }

    return render_items;
}
