use super::plane_data::PlaneMesh;
use crate::conversion::mesh_data::MeshData;
use crate::model::base::Vector3;
use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Clone, Debug)]
struct FaceGroup {
    indices: Vec<usize>,
    normal: Vector3,
}

fn optimize_plane_mesh(plane: &mut PlaneMesh) {
    let num_faces = plane.indices.len() / 3;
    let mut using_indices = vec![false; plane.positions.len() / 3];
    for i in 0..num_faces {
        let idx0 = plane.indices[i * 3] as usize;
        let idx1 = plane.indices[i * 3 + 1] as usize;
        let idx2 = plane.indices[i * 3 + 2] as usize;
        using_indices[idx0] = true;
        using_indices[idx1] = true;
        using_indices[idx2] = true;
    }
    let mut index_map = HashMap::new();
    let mut new_positions = Vec::new();
    let mut offset = 0;
    for (i, &used) in using_indices.iter().enumerate() {
        if used {
            index_map.insert(i as i32, offset as i32);
            new_positions.push(plane.positions[i * 3]);
            new_positions.push(plane.positions[i * 3 + 1]);
            new_positions.push(plane.positions[i * 3 + 2]);
            offset += 1;
        }
    }
    let mut new_indices = Vec::new();
    for idx in plane.indices.iter() {
        if let Some(&new_idx) = index_map.get(&idx) {
            new_indices.push(new_idx);
        } else {
            // assert!(false, "Index not found in index_map");
            // This should not happen if the input data is consistent
        }
    }
    plane.positions = new_positions;
    plane.indices = new_indices;
}

pub fn create_plane_meshes_from_mesh(mesh: &MeshData, threthould: f32) -> Vec<PlaneMesh> {
    let mut planes = Vec::new();
    let num_faces = mesh.indices.len() / 3;
    let mut face_normals = vec![None; num_faces];
    // Assuming each plane is defined by 4 vertices (2 triangles)
    for i in 0..num_faces {
        let idx0 = mesh.indices[i * 3] as usize;
        let idx1 = mesh.indices[i * 3 + 1] as usize;
        let idx2 = mesh.indices[i * 3 + 2] as usize;

        let v0 = Vector3::new(
            mesh.positions[idx0 * 3],
            mesh.positions[idx0 * 3 + 1],
            mesh.positions[idx0 * 3 + 2],
        );
        let v1 = Vector3::new(
            mesh.positions[idx1 * 3],
            mesh.positions[idx1 * 3 + 1],
            mesh.positions[idx1 * 3 + 2],
        );
        let v2 = Vector3::new(
            mesh.positions[idx2 * 3],
            mesh.positions[idx2 * 3 + 1],
            mesh.positions[idx2 * 3 + 2],
        );

        // Compute the normal of the face
        let edge1 = v1 - v0;
        let edge2 = v2 - v0;
        let normal = Vector3::cross(&edge1, &edge2);
        let sqr_length = Vector3::dot(&normal, &normal);
        if sqr_length > 0.0 {
            let normal = normal / sqr_length.sqrt();
            face_normals[i] = Some(normal);
        }
    }
    let mut face_groups = Vec::new();
    for i in 0..num_faces {
        if let Some(normal) = face_normals[i] {
            let face_group = FaceGroup {
                indices: vec![i],
                normal,
            };
            face_groups.push(RefCell::new(face_group));
        }
    }
    let num_groups = face_groups.len();
    for i in 0..num_groups {
        for j in (i + 1)..num_groups {
            let group_i = &face_groups[i];
            let group_j = &face_groups[j];
            let mut group_i = group_i.borrow_mut();
            let mut group_j = group_j.borrow_mut();
            if !group_i.indices.is_empty() || !group_j.indices.is_empty() {
                if Vector3::dot(&group_i.normal, &group_j.normal) > threthould {
                    let i_weitght = group_i.indices.len() as f32;
                    let j_weitght = group_j.indices.len() as f32;
                    let new_normal =
                        (group_i.normal * i_weitght + group_j.normal * j_weitght).normalize();
                    group_i.normal = new_normal;
                    group_i.indices.extend(&group_j.indices);
                    group_j.indices.clear();
                }
            }
        }
    }

    for group in face_groups {
        let group = group.borrow();
        if !group.indices.is_empty() {
            let mut plane_indices = Vec::new();
            let plane_positions = mesh.positions.clone();
            for &face_idx in &group.indices {
                let idx0 = mesh.indices[face_idx * 3] as usize;
                let idx1 = mesh.indices[face_idx * 3 + 1] as usize;
                let idx2 = mesh.indices[face_idx * 3 + 2] as usize;
                plane_indices.push(idx0 as i32);
                plane_indices.push(idx1 as i32);
                plane_indices.push(idx2 as i32);
            }
            let plane = PlaneMesh {
                indices: plane_indices,
                positions: plane_positions,
            };
            planes.push(plane);
        }
    }
    for plane in planes.iter_mut() {
        optimize_plane_mesh(plane);
    }
    return planes;
}
