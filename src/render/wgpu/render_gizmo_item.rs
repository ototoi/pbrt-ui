use super::lines::RenderLines;
use super::material::RenderCategory;
use super::material::RenderMaterial;
use super::material::RenderUniformValue;
use super::render_item::LinesRenderItem;
use super::render_item::RenderItem;
use super::render_resource::RenderResourceManager;
use crate::model::scene::CoordinateSystemComponent;
use crate::model::scene::Node;
use crate::model::scene::ResourceManager;
use crate::render::render_mode::RenderMode;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::wgpu;
use uuid::Uuid;

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
        render_type: RenderCategory::Opaque,
        uniform_values,
    };
    let render_material = Arc::new(render_material);
    render_resource_manager.add_material(&render_material);
    return Some(render_material);
}

pub fn get_render_axis_gizmo_items(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    _node: &Arc<RwLock<Node>>,
    _mode: RenderMode,
    _resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Vec<Arc<RenderItem>> {
    const IDS: [Uuid; 3] = [
        Uuid::from_u128(0x00000000_1000_0000_0000_000000000001), // X Axis
        Uuid::from_u128(0x00000000_1000_0000_0000_000000000002), // Y Axis
        Uuid::from_u128(0x00000000_1000_0000_0000_000000000003), // Z Axis
    ];
    let mut render_items = Vec::new();
    for i in 0..3 {
        let id = IDS[i];
        let edition = "world_axes".to_string();
        let mut render_lines = None;
        if let Some(lines) = render_resource_manager.get_lines(id) {
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
            if let Some(lines) = RenderLines::from_lines(device, queue, id, &edition, &lines) {
                let lines = Arc::new(lines);
                render_resource_manager.add_lines(&lines);
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
            let material = get_lines_material(id, &edition, render_resource_manager, &color);
            let render_item = LinesRenderItem {
                lines,
                material,
                matrix,
            };
            render_items.push(Arc::new(RenderItem::Lines(render_item)));
        }
    }
    return render_items;
}

pub fn get_render_grid_gizmo_items(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    node: &Arc<RwLock<Node>>,
    _mode: RenderMode,
    _resource_manager: &ResourceManager,
    render_resource_manager: &mut RenderResourceManager,
) -> Vec<Arc<RenderItem>> {
    const ID: Uuid = Uuid::from_u128(0x00000000_1000_0000_0000_000000000004); // Unique ID for the grid
    const GRID_SIZE: f32 = 1000.0; // Size of the grid
    const GRID_STEP: f32 = 10.0; // Step size for grid lines
    enum PlaneType {
        XY,
        ZX,
        YZ,
    }
    let mut render_items = Vec::new();
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
    if let Some(lines) = render_resource_manager.get_lines(id) {
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
            render_resource_manager.add_lines(&lines);
            render_lines = Some(lines);
        }
    }
    if let Some(lines) = render_lines {
        let color = [0.5, 0.5, 0.5, 1.0]; // Gray color for the grid
        let material = get_lines_material(id, &edition, render_resource_manager, &color);
        let matrix = glam::Mat4::IDENTITY; // Grid is at the origin
        let render_item = LinesRenderItem {
            lines,
            material,
            matrix,
        };
        render_items.push(Arc::new(RenderItem::Lines(render_item)));
    }
    return render_items;
}
