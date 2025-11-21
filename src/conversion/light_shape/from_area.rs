use std::vec;

use super::light_shape::LightShape;
use crate::conversion::mesh_data::create_mesh_data;
use crate::conversion::plane_data::PlaneMesh;
use crate::conversion::plane_data::create_plane_meshes_from_mesh;
use crate::conversion::plane_data::create_plane_outline_from_plane_mesh;
use crate::conversion::plane_data::create_plane_rect_from_plane_outline;
use crate::model::base::Vector3;
use crate::model::scene::Light;
use crate::model::scene::Shape;

#[derive(Debug, Clone, Copy)]
enum CreateLinesType {
    #[allow(dead_code)]
    FromMesh,
    #[allow(dead_code)]
    FromOutline,
    #[allow(dead_code)]
    FromRect,
}

fn create_lines_from_mesh(plane: &PlaneMesh) -> Option<Vec<Vector3>> {
    let mut lines = Vec::new();
    let num_faces = plane.indices.len() / 3;
    for i in 0..num_faces {
        let idx0 = plane.indices[i * 3] as usize;
        let idx1 = plane.indices[i * 3 + 1] as usize;
        let idx2 = plane.indices[i * 3 + 2] as usize;
        let v0 = Vector3::new(
            plane.positions[idx0 * 3],
            plane.positions[idx0 * 3 + 1],
            plane.positions[idx0 * 3 + 2],
        );
        let v1 = Vector3::new(
            plane.positions[idx1 * 3],
            plane.positions[idx1 * 3 + 1],
            plane.positions[idx1 * 3 + 2],
        );
        let v2 = Vector3::new(
            plane.positions[idx2 * 3],
            plane.positions[idx2 * 3 + 1],
            plane.positions[idx2 * 3 + 2],
        );
        lines.push(v0);
        lines.push(v1);
        lines.push(v2);
        lines.push(v0);
    }
    if !lines.is_empty() {
        return Some(lines);
    } else {
        return None;
    }
}

fn create_lines_from_outline(plane: &PlaneMesh) -> Option<Vec<Vector3>> {
    if let Some(outline) = create_plane_outline_from_plane_mesh(plane) {
        let positions = &outline.positions;
        let num_vertices = positions.len() / 3;
        let mut lines = Vec::new();
        for i in 0..num_vertices {
            let v = Vector3::new(positions[i * 3], positions[i * 3 + 1], positions[i * 3 + 2]);
            lines.push(v);
        }
        {
            let i = 0;
            let v = Vector3::new(positions[i * 3], positions[i * 3 + 1], positions[i * 3 + 2]);
            lines.push(v);
        }
        return Some(lines);
    }
    return None;
}

fn create_lines_from_rect(plane: &PlaneMesh) -> Option<Vec<Vec<Vector3>>> {
    if let Some(outline) = create_plane_outline_from_plane_mesh(plane) {
        if let Some(rect) = create_plane_rect_from_plane_outline(&outline, 0.99) {
            let mut total_lines = Vec::new();
            let center = Vector3::new(rect.position[0], rect.position[1], rect.position[2]);
            let u_axis = Vector3::new(rect.u_axis[0], rect.u_axis[1], rect.u_axis[2]);
            let v_axis = Vector3::new(rect.v_axis[0], rect.v_axis[1], rect.v_axis[2]);
            let corners = [
                center + u_axis + v_axis,
                center - u_axis + v_axis,
                center - u_axis - v_axis,
                center + u_axis - v_axis,
            ];
            let mut lines = Vec::new();
            for i in 0..5 {
                lines.push(corners[i % 4]);
            }
            total_lines.push(lines);
            let normal = Vector3::new(rect.normal[0], rect.normal[1], rect.normal[2]);
            let v1 = center;
            let v2 = center + 4.0 * normal;
            let lines = vec![v1, v2];
            total_lines.push(lines);
            return Some(total_lines);
        }
    }
    return None;
}

pub fn create_light_shape_from_mesh_area(_light: &Light, shape: &Shape) -> Option<LightShape> {
    if let Some(mesh) = create_mesh_data(shape) {
        let planes = create_plane_meshes_from_mesh(&mesh, 0.99);
        const CREATE_TYPE: CreateLinesType = CreateLinesType::FromRect;
        if !planes.is_empty() {
            let mut outlines = Vec::new();
            for plane in planes.iter() {
                match CREATE_TYPE {
                    CreateLinesType::FromMesh => {
                        if let Some(lines) = create_lines_from_mesh(plane) {
                            outlines.push(lines);
                        }
                    }
                    CreateLinesType::FromOutline => {
                        if let Some(lines) = create_lines_from_outline(plane) {
                            outlines.push(lines);
                        }
                    }
                    CreateLinesType::FromRect => {
                        if let Some(lines) = create_lines_from_rect(plane) {
                            outlines.extend(lines);
                        }
                    }
                }
            }
            let light_shape = LightShape { lines: outlines };
            return Some(light_shape);
        }
    }
    return None;
}

pub fn create_light_shape_from_area(light: &Light, shape: &Shape) -> Option<LightShape> {
    let shape_type = shape.get_type();
    match shape_type.as_str() {
        "trianglemesh" | "plymesh" => {
            return create_light_shape_from_mesh_area(light, shape);
        }
        _ => {
            return None;
        }
    }
}
